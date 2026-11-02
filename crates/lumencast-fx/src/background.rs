//! Background effects: blur, remove or replace everything that is not the
//! person, given a person matte. Our own implementation: the blur is a box
//! blur of a eighth-size copy, repeated twice, which is cheap and close
//! enough to a Gaussian for a background that is out of focus anyway.
//! NVIDIA Broadcast does the same three things (`background_blur`,
//! `background_remove`, `background_replace`) from a neural matte.

use crate::frame::Frame;
use crate::matte::Matte;
use crate::params::Strength;
use crate::stages::{resize_bilinear, Rect, Stage};
use rayon::prelude::*;

/// What goes behind the person.
#[derive(Debug, Clone)]
pub enum Backdrop {
    /// The same picture, out of focus.
    Blur(Strength),
    /// A flat colour (the "remove" mode).
    Solid([u8; 3]),
    /// Another picture, scaled to cover the frame.
    Image(Frame),
}

#[derive(Debug, Clone)]
pub struct Background {
    pub matte: Matte,
    pub backdrop: Backdrop,
}

/// The blur works on a copy this many times smaller (eight, so 160x90 for 720p).
const BLUR_SHRINK: u32 = 8;
/// Largest box radius, in pixels of the shrunk copy, at full strength.
const MAX_RADIUS: f32 = 8.0;

/// Edge firming: the model's matte is soft, so values are pushed away from the
/// middle between these two levels.
fn firm(alpha: u8) -> f32 {
    let t = ((f32::from(alpha) / 255.0 - 0.25) / 0.5).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn whole(frame: &Frame) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: frame.width(),
        height: frame.height(),
    }
}

/// One horizontal box blur pass with edge clamping, on `channels` interleaved.
fn box_blur_rows(data: &[u8], width: usize, height: usize, radius: usize) -> Vec<u8> {
    let mut out = vec![0u8; data.len()];
    let window = (2 * radius + 1) as u32;
    for y in 0..height {
        let row = &data[y * width * 3..(y + 1) * width * 3];
        let target = &mut out[y * width * 3..(y + 1) * width * 3];
        for c in 0..3 {
            let at = |x: isize| u32::from(row[(x.clamp(0, width as isize - 1) as usize) * 3 + c]);
            let mut sum: u32 = (-(radius as isize)..=radius as isize).map(at).sum();
            for x in 0..width {
                target[x * 3 + c] = (sum / window) as u8;
                sum = sum + at((x + radius + 1) as isize) - at(x as isize - radius as isize);
            }
        }
    }
    out
}

fn transpose(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; data.len()];
    for y in 0..height {
        for x in 0..width {
            let from = (y * width + x) * 3;
            let to = (x * height + y) * 3;
            out[to..to + 3].copy_from_slice(&data[from..from + 3]);
        }
    }
    out
}

/// A blurred copy of `frame`, at its own size.
fn blurred(frame: &Frame, strength: Strength) -> Frame {
    let (width, height) = (frame.width(), frame.height());
    let (small_w, small_h) = ((width / BLUR_SHRINK).max(1), (height / BLUR_SHRINK).max(1));
    let small = resize_bilinear(frame, whole(frame), small_w, small_h);
    let radius = ((strength.get() * MAX_RADIUS).round() as usize).max(1);
    let (w, h) = (small_w as usize, small_h as usize);
    let mut data = small.data().to_vec();
    for _ in 0..2 {
        data = box_blur_rows(&data, w, h, radius);
        data = transpose(&data, w, h);
        data = box_blur_rows(&data, h, w, radius);
        data = transpose(&data, h, w);
    }
    // The passes keep the size, so the buffer always fits; otherwise the
    // unblurred copy is the safe answer.
    match Frame::new(small_w, small_h, data) {
        Ok(shrunk) => resize_bilinear(&shrunk, whole(&shrunk), width, height),
        Err(_) => frame.clone(),
    }
}

/// `image` scaled to cover a `width` x `height` frame, cropping the excess.
pub fn cover(image: &Frame, width: u32, height: u32) -> Frame {
    let scale = (width as f32 / image.width() as f32).max(height as f32 / image.height() as f32);
    let crop_w = ((width as f32 / scale).round() as u32).clamp(1, image.width());
    let crop_h = ((height as f32 / scale).round() as u32).clamp(1, image.height());
    let window = Rect {
        x: (image.width() - crop_w) / 2,
        y: (image.height() - crop_h) / 2,
        width: crop_w,
        height: crop_h,
    };
    resize_bilinear(image, window, width, height)
}

impl Stage for Background {
    fn apply(&self, mut frame: Frame) -> Frame {
        let (width, height) = (frame.width(), frame.height());
        let behind = match &self.backdrop {
            Backdrop::Blur(strength) => Some(blurred(&frame, *strength)),
            Backdrop::Image(image) => Some(if (image.width(), image.height()) == (width, height) {
                image.clone()
            } else {
                cover(image, width, height)
            }),
            Backdrop::Solid(_) => None,
        };
        let solid = match self.backdrop {
            Backdrop::Solid(color) => color,
            _ => [0; 3],
        };
        // The matte, firmed, as 8-bit values on its own grid.
        let (grid_w, grid_h) = (self.matte.width(), self.matte.height());
        let grid: Vec<u32> = self
            .matte
            .alpha()
            .iter()
            .map(|a| (firm(*a) * 255.0).round() as u32)
            .collect();
        // Interpolation taps per column: both grid columns and a 0..=256 weight.
        let columns: Vec<(usize, usize, u32)> = (0..width)
            .map(|x| {
                let f = ((x as f32 + 0.5) / width as f32 * grid_w as f32 - 0.5)
                    .clamp(0.0, (grid_w - 1) as f32);
                let x0 = f.floor() as usize;
                (
                    x0,
                    (x0 + 1).min(grid_w - 1),
                    ((f - x0 as f32) * 256.0) as u32,
                )
            })
            .collect();
        let width_px = width as usize;
        let behind_data = behind.as_ref().map(Frame::data);
        frame
            .data_mut()
            .par_chunks_exact_mut(width_px * 3)
            .enumerate()
            .for_each(|(y, line)| {
                let f = ((y as f32 + 0.5) / height as f32 * grid_h as f32 - 0.5)
                    .clamp(0.0, (grid_h - 1) as f32);
                let y0 = f.floor() as usize;
                let y1 = (y0 + 1).min(grid_h - 1);
                let wy = ((f - y0 as f32) * 256.0) as u32;
                let (row0, row1) = (
                    &grid[y0 * grid_w..(y0 + 1) * grid_w],
                    &grid[y1 * grid_w..(y1 + 1) * grid_w],
                );
                let behind_line =
                    behind_data.map(|back| &back[y * width_px * 3..(y + 1) * width_px * 3]);
                for (x, (x0, x1, wx)) in columns.iter().enumerate() {
                    let top = row0[*x0] * (256 - wx) + row0[*x1] * wx;
                    let bottom = row1[*x0] * (256 - wx) + row1[*x1] * wx;
                    let alpha = (top * (256 - wy) + bottom * wy) >> 16;
                    if alpha >= 255 {
                        continue;
                    }
                    let at = x * 3;
                    for c in 0..3 {
                        let back = match behind_line {
                            Some(back) => i32::from(back[at + c]),
                            None => i32::from(solid[c]),
                        };
                        let front = i32::from(line[at + c]);
                        line[at + c] = (back + ((front - back) * alpha as i32) / 255) as u8;
                    }
                }
            });
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stripes() -> Frame {
        Frame::from_fn(64, 48, |x, _| if x % 2 == 0 { [255; 3] } else { [0; 3] })
    }

    fn strength(value: f32) -> Strength {
        Strength::new(value).unwrap()
    }

    #[test]
    fn a_full_person_matte_leaves_the_picture_alone() {
        let frame = stripes();
        let out = Background {
            matte: Matte::uniform(8, 8, 255),
            backdrop: Backdrop::Solid([0, 200, 0]),
        }
        .apply(frame.clone());
        assert_eq!(out.data(), frame.data());
    }

    #[test]
    fn an_empty_matte_shows_only_the_backdrop() {
        let out = Background {
            matte: Matte::uniform(8, 8, 0),
            backdrop: Backdrop::Solid([10, 200, 30]),
        }
        .apply(stripes());
        assert_eq!(out.pixel(5, 5), [10, 200, 30]);
        assert_eq!(out.pixel(40, 40), [10, 200, 30]);
    }

    #[test]
    fn the_person_stays_sharp_and_the_background_goes_soft() {
        // Left half person, right half background.
        let matte = Matte::new(
            8,
            8,
            (0..64).map(|i| if i % 8 < 4 { 255 } else { 0 }).collect(),
        )
        .unwrap();
        let out = Background {
            matte,
            backdrop: Backdrop::Blur(strength(1.0)),
        }
        .apply(stripes());
        // Sharp on the left: neighbours still alternate fully.
        assert_eq!((out.pixel(2, 20)[0], out.pixel(3, 20)[0]), (255, 0));
        // Blurred on the right: neighbours are almost the same grey.
        let (a, b) = (out.pixel(50, 20)[0], out.pixel(51, 20)[0]);
        assert!(a.abs_diff(b) < 40, "{a} {b}");
    }

    #[test]
    fn a_replacement_picture_is_scaled_to_cover_the_frame() {
        let wide = Frame::from_fn(
            200,
            50,
            |x, _| if x < 100 { [255, 0, 0] } else { [0, 0, 255] },
        );
        let out = Background {
            matte: Matte::uniform(8, 8, 0),
            backdrop: Backdrop::Image(wide),
        }
        .apply(stripes());
        assert_eq!((out.width(), out.height()), (64, 48));
        assert!(out.pixel(2, 24)[0] > 200 && out.pixel(61, 24)[2] > 200);
    }

    #[test]
    fn a_stronger_blur_removes_more_detail() {
        let frame = stripes();
        let soft = |s| {
            let out = Background {
                matte: Matte::uniform(8, 8, 0),
                backdrop: Backdrop::Blur(strength(s)),
            }
            .apply(frame.clone());
            let (a, b) = (out.pixel(30, 20)[0], out.pixel(31, 20)[0]);
            a.abs_diff(b)
        };
        assert!(soft(1.0) <= soft(0.1));
    }

    #[test]
    fn small_and_odd_sizes_do_not_panic() {
        for (w, h) in [(1, 1), (3, 2), (7, 5)] {
            let out = Background {
                matte: Matte::uniform(4, 4, 100),
                backdrop: Backdrop::Blur(strength(0.5)),
            }
            .apply(Frame::from_fn(w, h, |_, _| [9; 3]));
            assert_eq!((out.width(), out.height()), (w, h));
        }
    }
}
