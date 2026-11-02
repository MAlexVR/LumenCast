//! The deterministic stages of slice B2.

use crate::frame::Frame;
use crate::params::{Pan, Strength, Zoom};
use rayon::prelude::*;

/// A frame transformation. Stages consume the frame so that identity and
/// in-place stages cost nothing.
pub trait Stage: Send + Sync {
    fn apply(&self, frame: Frame) -> Frame;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// The crop window of the legacy `applyZoom`: `width / zoom` rounded (at
/// least 1), shifted by `pan` inside the margin that zooming leaves.
pub fn crop_window(width: u32, height: u32, zoom: Zoom, pan_x: Pan, pan_y: Pan) -> Rect {
    let (crop_w, x) = axis(width, zoom, pan_x);
    let (crop_h, y) = axis(height, zoom, pan_y);
    Rect {
        x,
        y,
        width: crop_w,
        height: crop_h,
    }
}

/// Crop length and start along one axis.
fn axis(length: u32, zoom: Zoom, pan: Pan) -> (u32, u32) {
    let crop = (((length as f32 / zoom.get()).round() as u32).max(1)).min(length);
    let max_shift = (length - crop) / 2;
    let start = i64::from(max_shift) + (pan.get() * max_shift as f32).round() as i64;
    (crop, start.clamp(0, i64::from(length - crop)) as u32)
}

/// Manual zoom and pan: crop a window and scale it back to the full size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ManualFraming {
    pub zoom: Zoom,
    pub pan_x: Pan,
    pub pan_y: Pan,
}

impl Stage for ManualFraming {
    fn apply(&self, frame: Frame) -> Frame {
        if self.zoom.get() <= 1.0 {
            return frame;
        }
        let window = crop_window(
            frame.width(),
            frame.height(),
            self.zoom,
            self.pan_x,
            self.pan_y,
        );
        resize_bilinear(&frame, window, frame.width(), frame.height())
    }
}

/// One output sample position: where the two source elements start and the
/// weight of the second one. `unit` is the byte length of one source element
/// (3 for a pixel, a whole row for a line).
struct Tap {
    first: usize,
    second: usize,
    weight: f32,
}

/// Pixel-centre mapping with edge clamping, the convention of OpenCV's
/// `INTER_LINEAR`, so results match the legacy server to within rounding.
fn taps(offset: u32, source_len: u32, out_len: u32, unit: usize) -> Vec<Tap> {
    let scale = source_len as f32 / out_len as f32;
    let last = (source_len - 1) as f32;
    (0..out_len)
        .map(|index| {
            let position = ((index as f32 + 0.5) * scale - 0.5).clamp(0.0, last);
            let first = position.floor() as u32;
            Tap {
                first: (offset + first) as usize * unit,
                second: (offset + (first + 1).min(source_len - 1)) as usize * unit,
                weight: position - first as f32,
            }
        })
        .collect()
}

/// Horizontal pass over one source row, writing `out_width * 3` samples.
fn blend_row(source_row: &[u8], columns: &[Tap], out: &mut [f32]) {
    for (pixel, column) in out.chunks_exact_mut(3).zip(columns) {
        let (a, b) = (
            &source_row[column.first..column.first + 3],
            &source_row[column.second..column.second + 3],
        );
        for channel in 0..3 {
            let left = f32::from(a[channel]);
            pixel[channel] = left + (f32::from(b[channel]) - left) * column.weight;
        }
    }
}

/// Two passes: each needed source row is blended horizontally once and kept
/// for the next output row, then rows are blended vertically over contiguous
/// memory, which the compiler can vectorize.
/// `frame` scaled to `width` x `height` with bilinear sampling.
pub fn resize(frame: &Frame, width: u32, height: u32) -> Frame {
    let whole = Rect {
        x: 0,
        y: 0,
        width: frame.width(),
        height: frame.height(),
    };
    resize_bilinear(frame, whole, width.max(1), height.max(1))
}

pub(crate) fn resize_bilinear(
    source: &Frame,
    window: Rect,
    out_width: u32,
    out_height: u32,
) -> Frame {
    let columns = taps(window.x, window.width, out_width, 3);
    let stride = source.width() as usize * 3;
    let rows = taps(window.y, window.height, out_height, 1);
    let src = source.data();
    let row_len = out_width as usize * 3;
    let mut data = vec![0u8; row_len * out_height as usize];
    // Bands of rows are independent: each keeps its own pair of blended source
    // rows, so the only repeated work is one source row at a band's start.
    let band = BAND_ROWS.max(1);
    data.par_chunks_mut(row_len * band)
        .zip(rows.par_chunks(band))
        .for_each(|(out, band_rows)| {
            resize_rows(src, stride, &columns, band_rows, out, row_len);
        });
    Frame::from_fn_data(out_width, out_height, data)
}

/// Output rows per parallel task.
const BAND_ROWS: usize = 48;

/// Fills `out` (whole rows of `row_len` bytes) for the given row taps.
fn resize_rows(
    src: &[u8],
    stride: usize,
    columns: &[Tap],
    rows: &[Tap],
    out: &mut [u8],
    row_len: usize,
) {
    // The two most recent horizontally blended rows and the source row they hold.
    let mut held: [(usize, Vec<f32>); 2] = [
        (usize::MAX, vec![0.0; row_len]),
        (usize::MAX, vec![0.0; row_len]),
    ];
    let mut next_slot = 0;
    let mut fetch = |held: &mut [(usize, Vec<f32>); 2], index: usize| -> usize {
        if let Some(slot) = held.iter().position(|(held_index, _)| *held_index == index) {
            return slot;
        }
        let slot = next_slot;
        next_slot = 1 - next_slot;
        held[slot].0 = index;
        blend_row(
            &src[index * stride..(index + 1) * stride],
            columns,
            &mut held[slot].1,
        );
        slot
    };
    for (out_row, row) in out.chunks_exact_mut(row_len).zip(rows) {
        let upper = fetch(&mut held, row.first);
        let lower = fetch(&mut held, row.second);
        // `fetch` may have evicted `upper` when the two rows differ and both are new.
        let upper = if held[upper].0 == row.first {
            upper
        } else {
            fetch(&mut held, row.first)
        };
        let (top, bottom) = (&held[upper].1, &held[lower].1);
        for ((out, t), b) in out_row.iter_mut().zip(top).zip(bottom) {
            *out = (t + (b - t) * row.weight + 0.5) as u8;
        }
    }
}

/// Horizontal flip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mirror;

impl Stage for Mirror {
    fn apply(&self, mut frame: Frame) -> Frame {
        let row_len = frame.width() as usize * 3;
        for row in frame.data_mut().chunks_exact_mut(row_len) {
            let width = row.len() / 3;
            for x in 0..width / 2 {
                let (left, right) = (x * 3, (width - 1 - x) * 3);
                for channel in 0..3 {
                    row.swap(left + channel, right + channel);
                }
            }
        }
        frame
    }
}

/// Darkens towards the corners: the factor is `1 - strength * d²/dmax²`, where
/// `d` is the distance to the frame centre and `dmax` the distance to a corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vignette {
    pub strength: Strength,
}

impl Stage for Vignette {
    fn apply(&self, frame: Frame) -> Frame {
        let centre_x = (frame.width() - 1) as f32 / 2.0;
        let centre_y = (frame.height() - 1) as f32 / 2.0;
        vignette_around(frame, self.strength.get(), centre_x, centre_y)
    }
}

/// The same darkening, centred on a point of the frame (given as fractions of
/// its width and height) instead of the middle, so it can follow a face. The
/// farthest corner is the one that gets the full strength.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceVignette {
    pub strength: Strength,
    /// 0..=1 along each axis.
    pub centre: (f32, f32),
}

impl Stage for FaceVignette {
    fn apply(&self, frame: Frame) -> Frame {
        let centre_x = self.centre.0.clamp(0.0, 1.0) * (frame.width() - 1) as f32;
        let centre_y = self.centre.1.clamp(0.0, 1.0) * (frame.height() - 1) as f32;
        vignette_around(frame, self.strength.get(), centre_x, centre_y)
    }
}

fn vignette_around(mut frame: Frame, strength: f32, centre_x: f32, centre_y: f32) -> Frame {
    let far_x = centre_x.max((frame.width() - 1) as f32 - centre_x);
    let far_y = centre_y.max((frame.height() - 1) as f32 - centre_y);
    let corner = far_x * far_x + far_y * far_y;
    if strength == 0.0 || corner == 0.0 {
        return frame;
    }
    let k = strength / corner;
    let width = frame.width() as usize;
    let column_term: Vec<f32> = (0..width)
        .map(|x| {
            let dx = x as f32 - centre_x;
            k * dx * dx
        })
        .collect();
    frame
        .data_mut()
        .par_chunks_exact_mut(width * 3)
        .enumerate()
        .for_each(|(y, row)| {
            let dy = y as f32 - centre_y;
            let row_base = 1.0 - k * dy * dy;
            for (pixel, term) in row.chunks_exact_mut(3).zip(&column_term) {
                // 16-bit fixed-point factor in [0, 65536].
                let factor = ((row_base - term) * 65536.0 + 0.5).clamp(0.0, 65536.0) as u32;
                for channel in pixel {
                    *channel = ((u32::from(*channel) * factor + 32768) >> 16) as u8;
                }
            }
        });
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zoom(value: f32) -> Zoom {
        Zoom::new(value).unwrap()
    }

    fn pan(value: f32) -> Pan {
        Pan::new(value).unwrap()
    }

    fn gray(value: u8) -> [u8; 3] {
        [value; 3]
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn crop_window_matches_the_legacy_integer_math() {
        // No zoom: the whole frame.
        assert_eq!(
            crop_window(1280, 720, zoom(1.0), pan(0.0), pan(0.0)),
            rect(0, 0, 1280, 720)
        );
        // Zoom 2 centred: 640x360 starting at the middle of the margin.
        assert_eq!(
            crop_window(1280, 720, zoom(2.0), pan(0.0), pan(0.0)),
            rect(320, 180, 640, 360)
        );
        // Pan extremes reach the frame edges; half pan is rounded like std::round.
        assert_eq!(
            crop_window(1280, 720, zoom(2.0), pan(-1.0), pan(-1.0)),
            rect(0, 0, 640, 360)
        );
        assert_eq!(
            crop_window(1280, 720, zoom(2.0), pan(1.0), pan(1.0)),
            rect(640, 360, 640, 360)
        );
        assert_eq!(
            crop_window(1280, 720, zoom(2.0), pan(0.5), pan(0.0)),
            rect(480, 180, 640, 360)
        );
        // Odd sizes: crop 667 of 1001, margin (1001-667)/2 = 167 with integer division.
        assert_eq!(
            crop_window(1001, 501, zoom(1.5), pan(0.0), pan(0.0)),
            rect(167, 83, 667, 334)
        );
    }

    #[test]
    fn crop_window_never_leaves_the_frame() {
        for z in [1.0, 1.15, 1.5, 1.99, 2.0] {
            for p in [-1.0, -0.3, 0.0, 0.7, 1.0] {
                let r = crop_window(641, 479, zoom(z), pan(p), pan(-p));
                assert!(r.width >= 1 && r.height >= 1);
                assert!(r.x + r.width <= 641 && r.y + r.height <= 479, "{r:?}");
            }
        }
    }

    #[test]
    fn framing_without_zoom_returns_the_frame_untouched() {
        let frame = Frame::from_fn(8, 6, |x, y| [x as u8 * 10, y as u8 * 10, 7]);
        let stage = ManualFraming {
            zoom: zoom(1.0),
            pan_x: pan(0.7),
            pan_y: pan(-0.2),
        };
        assert_eq!(stage.apply(frame.clone()), frame);
    }

    #[test]
    fn framing_keeps_the_frame_size_and_flat_colours() {
        let frame = Frame::from_fn(16, 9, |_, _| [200, 100, 50]);
        let out = ManualFraming {
            zoom: zoom(1.7),
            pan_x: pan(0.3),
            pan_y: pan(0.9),
        }
        .apply(frame);
        assert_eq!((out.width(), out.height()), (16, 9));
        assert!(out.data().chunks(3).all(|px| px == [200, 100, 50]));
    }

    #[test]
    fn framing_resamples_like_a_bilinear_resize_of_the_crop() {
        // 4x4, value = 10 * x. Zoom 2 crops columns 1..=2 (values 10 and 20) and
        // scales 2 -> 4 with pixel-centre mapping: sources -0.25, 0.25, 0.75, 1.25
        // clamped to 0, 0.25, 0.75, 1, giving 10, 12.5, 17.5, 20.
        let frame = Frame::from_fn(4, 4, |x, _| gray(x as u8 * 10));
        let out = ManualFraming {
            zoom: zoom(2.0),
            pan_x: pan(0.0),
            pan_y: pan(0.0),
        }
        .apply(frame);
        let row: Vec<u8> = (0..4).map(|x| out.pixel(x, 2)[0]).collect();
        let expected = [10u8, 13, 18, 20];
        for (got, want) in row.iter().zip(expected) {
            assert!(got.abs_diff(want) <= 1, "{row:?}");
        }
    }

    #[test]
    fn panning_moves_the_window_towards_the_requested_edge() {
        let frame = Frame::from_fn(40, 4, |x, _| gray(x as u8 * 6));
        let left = ManualFraming {
            zoom: zoom(2.0),
            pan_x: pan(-1.0),
            pan_y: pan(0.0),
        }
        .apply(frame.clone());
        let right = ManualFraming {
            zoom: zoom(2.0),
            pan_x: pan(1.0),
            pan_y: pan(0.0),
        }
        .apply(frame);
        assert_eq!(left.pixel(0, 0)[0], 0);
        assert!(left.pixel(39, 0)[0] <= 6 * 20);
        assert!(right.pixel(0, 0)[0] >= 6 * 20 - 1);
        assert_eq!(right.pixel(39, 0)[0], 6 * 39);
    }

    #[test]
    fn mirror_flips_horizontally_and_twice_is_the_identity() {
        let frame = Frame::from_fn(5, 3, |x, y| [x as u8, y as u8, 9]);
        let flipped = Mirror.apply(frame.clone());
        for y in 0..3 {
            for x in 0..5 {
                assert_eq!(flipped.pixel(x, y), frame.pixel(4 - x, y));
            }
        }
        assert_eq!(Mirror.apply(flipped), frame);
    }

    #[test]
    fn vignette_with_zero_strength_changes_nothing() {
        let frame = Frame::from_fn(7, 5, |x, y| [x as u8 * 9, y as u8 * 9, 200]);
        let stage = Vignette {
            strength: Strength::new(0.0).unwrap(),
        };
        assert_eq!(stage.apply(frame.clone()), frame);
    }

    #[test]
    fn vignette_follows_the_documented_falloff() {
        // 3x3 grey 200, centre (1,1): centre factor 1, edge midpoint d²=1 of 2
        // so 0.5 at full strength, corner factor 0.
        let frame = Frame::from_fn(3, 3, |_, _| gray(200));
        let out = Vignette {
            strength: Strength::new(1.0).unwrap(),
        }
        .apply(frame);
        assert_eq!(out.pixel(1, 1), gray(200));
        assert_eq!(out.pixel(1, 0), gray(100));
        assert_eq!(out.pixel(0, 1), gray(100));
        assert_eq!(out.pixel(0, 0), gray(0));
        assert_eq!(out.pixel(2, 2), gray(0));
    }

    #[test]
    fn vignette_is_symmetric_and_grows_with_strength() {
        let frame = Frame::from_fn(9, 9, |_, _| gray(240));
        let soft = Vignette {
            strength: Strength::new(0.3).unwrap(),
        }
        .apply(frame.clone());
        let hard = Vignette {
            strength: Strength::new(0.9).unwrap(),
        }
        .apply(frame);
        assert_eq!(soft.pixel(0, 0), soft.pixel(8, 8));
        assert_eq!(soft.pixel(0, 8), soft.pixel(8, 0));
        assert_eq!(soft.pixel(2, 4), soft.pixel(6, 4));
        assert!(hard.pixel(0, 0)[0] < soft.pixel(0, 0)[0]);
        assert!(soft.pixel(0, 0)[0] < 240);
        assert_eq!(soft.pixel(4, 4), gray(240));
    }

    #[test]
    fn a_vignette_centred_on_the_middle_equals_the_plain_one() {
        let frame = Frame::from_fn(9, 7, |x, y| [(x * 20) as u8, (y * 30) as u8, 150]);
        let strength = Strength::new(0.6).unwrap();
        assert_eq!(
            FaceVignette {
                strength,
                centre: (0.5, 0.5)
            }
            .apply(frame.clone()),
            Vignette { strength }.apply(frame)
        );
    }

    #[test]
    fn a_vignette_follows_its_centre_and_leaves_that_spot_untouched() {
        let frame = Frame::from_fn(21, 11, |_, _| gray(220));
        let out = FaceVignette {
            strength: Strength::new(1.0).unwrap(),
            centre: (0.25, 0.5),
        }
        .apply(frame);
        // The centre sits at x = 5: untouched, and the same distance to the left
        // or to the right darkens equally, but the far right is darker than the near left.
        assert_eq!(out.pixel(5, 5), gray(220));
        assert_eq!(out.pixel(2, 5), out.pixel(8, 5));
        assert!(out.pixel(20, 5)[0] < out.pixel(0, 5)[0]);
        assert_eq!(
            out.pixel(20, 0),
            gray(0),
            "the far corner gets the full strength"
        );
    }
}
