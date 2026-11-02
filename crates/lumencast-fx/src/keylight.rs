//! Virtual key light: a soft, face-anchored lift of the shadows, like a lamp
//! aimed at the user. It is our own design, not a copy of anything: the light
//! falloff is a closed-form function of the distance to the face (no blur
//! pass), the lift adapts to how dark the face is, and the warmth tints the
//! light from cool to warm. NVIDIA Broadcast's `virtual_key_light` relights
//! with a neural model and an environment map; this is a CPU approximation of
//! the visible effect, not an equivalent of that model.

use crate::frame::Frame;
use crate::lightprobe::LightProbe;
use crate::params::Strength;
use crate::reframe::FaceBox;
use crate::stages::Stage;
use rayon::prelude::*;

/// Luma the face is lifted towards when it is darker than this.
const TARGET_LUMA: f32 = 200.0;
/// The most a dark face is brightened, so highlights never blow out.
const MAX_RATIO: f32 = 2.4;
/// Constant lift at full intensity, in levels.
const BASE_LIFT: f32 = 10.0;
/// Colour shift at the warmest or coolest setting, in levels.
const MAX_TINT: f32 = 12.0;
/// The light covers a bit more than the face: this many face heights tall and
/// this many face widths wide (a face is about as wide as it is tall).
const REACH_X: f32 = 1.4;
const REACH_Y: f32 = 2.0;
/// Distance (in ellipse units) where the light is at full strength and where
/// it has faded out.
const FULL_UNTIL: f32 = 0.45;
const FADED_AT: f32 = 1.15;
/// The face luma is measured on every Nth pixel of the lit area.
const SAMPLE_STEP: usize = 4;
/// The slider is not linear: a studio light has to be seen at low settings too,
/// so intensity is raised to this power (below one) before it is used.
const INTENSITY_CURVE: f32 = 0.7;
/// A key light comes from one side: the side it comes from (the left of the
/// picture) gets this much more of it and the other side this much less.
const SIDE_BIAS: f32 = 0.4;
/// How much more of a light picture's light the side it comes from gets, and
/// the top or bottom it comes from, at full focus.
const PROBE_BIAS: f32 = 0.55;
const PROBE_BIAS_Y: f32 = 0.4;
/// Brightness above which highlights are compressed instead of clipped.
const SHOULDER_START: f32 = 205.0;
/// Entries of the falloff table.
const TABLE_SIZE: usize = 1024;

#[derive(Debug, Clone, Copy)]
pub struct KeyLight {
    pub face: FaceBox,
    pub intensity: Strength,
    /// 0 is a cool light, 1 a warm one.
    pub warmth: Strength,
    /// A light picture: where the light comes from and its colour, instead of
    /// the fixed lamp on the left.
    pub probe: Option<LightProbe>,
}

/// Maps a brightened value (0..=1023) to 0..=255: unchanged up to the shoulder,
/// then compressed smoothly so highlights roll off instead of clipping.
fn shoulder_table() -> Vec<i32> {
    let room = 255.0 - SHOULDER_START;
    (0..1024)
        .map(|v| {
            let v = v as f32;
            if v <= SHOULDER_START {
                v as i32
            } else {
                (SHOULDER_START + room * (1.0 - (-(v - SHOULDER_START) / room).exp())).round()
                    as i32
            }
        })
        .collect()
}

/// 1 inside, 0 outside, smooth in between.
fn falloff(distance: f32) -> f32 {
    let t = ((FADED_AT - distance) / (FADED_AT - FULL_UNTIL)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl KeyLight {
    /// The pixel box the light can touch, clamped to the frame.
    fn bounds(&self, width: u32, height: u32) -> Option<(usize, usize, usize, usize)> {
        let rx = self.face.height * REACH_X * FADED_AT;
        let ry = self.face.height * REACH_Y * FADED_AT;
        let cy = self.face.center_y + self.face.height * 0.15;
        let left = (self.face.center_x - rx).floor().max(0.0);
        let right = (self.face.center_x + rx).ceil().min(width as f32);
        let top = (cy - ry).floor().max(0.0);
        let bottom = (cy + ry).ceil().min(height as f32);
        (right > left && bottom > top).then_some((
            left as usize,
            right as usize,
            top as usize,
            bottom as usize,
        ))
    }
}

impl Stage for KeyLight {
    fn apply(&self, mut frame: Frame) -> Frame {
        let intensity = self.intensity.get().powf(INTENSITY_CURVE);
        if intensity <= 0.0 || self.face.height < 1.0 {
            return frame;
        }
        let width = frame.width() as usize;
        let Some((left, right, top, bottom)) = self.bounds(frame.width(), frame.height()) else {
            return frame;
        };
        let rx = self.face.height * REACH_X;
        let ry = self.face.height * REACH_Y;
        let (cx, cy) = (
            self.face.center_x,
            self.face.center_y + self.face.height * 0.15,
        );
        // Squared distance splits into an x part and a y part, so each is
        // computed once per column and per row, and the falloff comes from a
        // table indexed by the squared distance.
        let columns: Vec<f32> = (left..right)
            .map(|x| {
                let dx = (x as f32 + 0.5 - cx) / rx;
                dx * dx
            })
            .collect();
        let limit = FADED_AT * FADED_AT;
        let table: Vec<u16> = (0..=TABLE_SIZE)
            .map(|i| {
                let squared = limit * i as f32 / TABLE_SIZE as f32;
                (falloff(squared.sqrt()) * 256.0).round() as u16
            })
            .collect();
        let weight_at = |column: f32, row: f32| -> u16 {
            let squared = column + row;
            if squared >= limit {
                0
            } else {
                table[(squared / limit * TABLE_SIZE as f32) as usize]
            }
        };
        let row_of = |y: usize| {
            let dy = (y as f32 + 0.5 - cy) / ry;
            dy * dy
        };

        // How dark is the lit face? A weighted mean on a coarse grid is enough.
        let data = frame.data();
        let (mut sum, mut total) = (0.0_f32, 0.0_f32);
        for y in (top..bottom).step_by(SAMPLE_STEP) {
            let row = row_of(y);
            for x in (left..right).step_by(SAMPLE_STEP) {
                let w = f32::from(weight_at(columns[x - left], row));
                if w > 0.0 {
                    let at = (y * width + x) * 3;
                    let luma = 0.299 * f32::from(data[at])
                        + 0.587 * f32::from(data[at + 1])
                        + 0.114 * f32::from(data[at + 2]);
                    sum += luma * w;
                    total += w;
                }
            }
        }
        if total <= 0.0 {
            return frame;
        }
        let mean = (sum / total).max(1.0);
        let ratio = (TARGET_LUMA / mean).clamp(1.0, MAX_RATIO);
        let gain = 1.0 + (ratio - 1.0) * intensity;
        let lift = BASE_LIFT * intensity;
        // A light picture shifts the warmth the slider sets towards its own colour.
        let warmth = match self.probe {
            Some(probe) => (self.warmth.get() + 0.5 * probe.warmth).clamp(0.0, 1.0),
            None => self.warmth.get(),
        };
        let tint = (warmth - 0.5) * 2.0 * MAX_TINT * intensity;
        let shoulder = shoulder_table();
        // How much of the light each column gets: more on the side it comes from.
        // Without a picture it comes from the left; with one, from wherever the
        // picture is brightest, as strongly as it is directional.
        let (side_bias, along) = match self.probe {
            Some(probe) => (PROBE_BIAS * probe.focus, probe.x),
            None => (SIDE_BIAS, -1.0),
        };
        let side: Vec<i32> = (left..right)
            .map(|x| {
                let across = ((x as f32 + 0.5 - cx) / rx).clamp(-1.0, 1.0);
                ((1.0 + side_bias * across * along).max(0.0) * 256.0).round() as i32
            })
            .collect();
        // Up and down only matters with a picture: a light from above leaves
        // the chin darker, one from below the forehead.
        let vertical = |y: usize| -> i32 {
            match self.probe {
                Some(probe) => {
                    let down = ((y as f32 + 0.5 - cy) / ry).clamp(-1.0, 1.0);
                    ((1.0 + PROBE_BIAS_Y * probe.focus * down * probe.y).max(0.0) * 256.0).round()
                        as i32
                }
                None => 256,
            }
        };
        // Fixed point, 8 fractional bits: gain and per-channel offsets.
        let gain_fp = (gain * 256.0).round() as i32;
        let offsets = [
            ((lift + tint) * 256.0).round() as i32,
            (lift * 256.0).round() as i32,
            ((lift - tint) * 256.0).round() as i32,
        ];

        frame
            .data_mut()
            .par_chunks_exact_mut(width * 3)
            .enumerate()
            .skip(top)
            .take(bottom - top)
            .for_each(|(y, full_line)| {
                let row = row_of(y);
                let row_bias = vertical(y);
                let line = &mut full_line[left * 3..right * 3];
                for ((pixel, column), bias) in line.chunks_exact_mut(3).zip(&columns).zip(&side) {
                    let w = ((i32::from(weight_at(*column, row)) * bias * row_bias) >> 16).min(256);
                    if w == 0 {
                        continue;
                    }
                    for (value, offset) in pixel.iter_mut().zip(offsets) {
                        let old = i32::from(*value);
                        // A light never darkens: what the shoulder would pull below the
                        // original stays as it was.
                        let lit = shoulder[((old * gain_fp + offset) >> 8).clamp(0, 1023) as usize]
                            .max(old);
                        *value = (old + (((lit - old) * w) >> 8)).clamp(0, 255) as u8;
                    }
                }
            });
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dark(width: u32, height: u32) -> Frame {
        Frame::from_fn(width, height, |_, _| [60, 60, 60])
    }

    fn light(face: FaceBox, intensity: f32, warmth: f32) -> KeyLight {
        KeyLight {
            face,
            intensity: Strength::new(intensity).unwrap(),
            warmth: Strength::new(warmth).unwrap(),
            probe: None,
        }
    }

    fn face(center_x: f32, center_y: f32, height: f32) -> FaceBox {
        FaceBox {
            center_x,
            center_y,
            height,
        }
    }

    #[test]
    fn a_dark_face_is_lifted_and_the_far_corners_are_left_alone() {
        let out = light(face(320.0, 240.0, 100.0), 1.0, 0.5).apply(dark(640, 480));
        assert!(out.pixel(320, 260)[1] > 80, "{:?}", out.pixel(320, 260));
        assert_eq!(out.pixel(5, 5), [60, 60, 60]);
        assert_eq!(out.pixel(634, 474), [60, 60, 60]);
    }

    #[test]
    fn zero_intensity_changes_nothing() {
        let frame = dark(64, 48);
        let out = light(face(32.0, 24.0, 20.0), 0.0, 0.5).apply(frame.clone());
        assert_eq!(out.data(), frame.data());
    }

    #[test]
    fn a_stronger_light_lifts_more() {
        let weak = light(face(320.0, 240.0, 100.0), 0.3, 0.5).apply(dark(640, 480));
        let strong = light(face(320.0, 240.0, 100.0), 1.0, 0.5).apply(dark(640, 480));
        assert!(strong.pixel(320, 240)[1] > weak.pixel(320, 240)[1]);
    }

    #[test]
    fn an_already_bright_face_gains_far_less_than_a_dark_one_and_does_not_clip() {
        let at = |level: u8| {
            let frame = Frame::from_fn(640, 480, |_, _| [level; 3]);
            light(face(320.0, 240.0, 100.0), 1.0, 0.5)
                .apply(frame)
                .pixel(320, 240)[1]
        };
        let (bright, dark) = (at(200), at(60));
        assert!(
            bright >= 200 && bright - 200 <= 14,
            "bright went to {bright}"
        );
        assert!(dark - 60 >= 60, "dark went to {dark}");
        assert!(
            at(250) >= 250,
            "highlights roll off, they do not drop or clip"
        );
    }

    #[test]
    fn warmth_moves_the_colour_between_cool_and_warm() {
        let warm = light(face(320.0, 240.0, 100.0), 1.0, 1.0).apply(dark(640, 480));
        let cool = light(face(320.0, 240.0, 100.0), 1.0, 0.0).apply(dark(640, 480));
        let (w, c) = (warm.pixel(320, 240), cool.pixel(320, 240));
        assert!(w[0] > w[2], "warm: {w:?}");
        assert!(c[2] > c[0], "cool: {c:?}");
    }

    #[test]
    fn a_face_at_the_edge_or_outside_the_frame_does_not_panic() {
        for (x, y) in [(0.0, 0.0), (639.0, 479.0), (-500.0, 240.0), (320.0, 5000.0)] {
            let out = light(face(x, y, 100.0), 1.0, 0.5).apply(dark(640, 480));
            assert_eq!(out.width(), 640);
        }
        let tiny = light(face(1.0, 1.0, 0.2), 1.0, 0.5).apply(dark(4, 4));
        assert_eq!(tiny.height(), 4);
    }

    fn probe(x: f32, y: f32, warmth: f32) -> Option<LightProbe> {
        Some(LightProbe {
            x,
            y,
            warmth,
            focus: 1.0,
        })
    }

    /// Mean of channel `c` over a box of the frame.
    fn mean(frame: &Frame, c: usize, x0: u32, x1: u32, y0: u32, y1: u32) -> f32 {
        let mut sum = 0.0;
        let mut n = 0.0;
        for y in y0..y1 {
            for x in x0..x1 {
                sum += f32::from(frame.pixel(x, y)[c]);
                n += 1.0;
            }
        }
        sum / n
    }

    #[test]
    fn a_light_picture_decides_the_side_the_light_comes_from() {
        let f = face(200.0, 200.0, 120.0);
        let lit = |x: f32| {
            let mut light = light(f, 1.0, 0.5);
            light.probe = probe(x, 0.0, 0.0);
            light.apply(dark(400, 400))
        };
        let (from_left, from_right) = (lit(-1.0), lit(1.0));
        let left = |frame: &Frame| mean(frame, 1, 140, 190, 160, 240);
        let right = |frame: &Frame| mean(frame, 1, 210, 260, 160, 240);
        assert!(left(&from_left) > right(&from_left) + 2.0);
        assert!(right(&from_right) > left(&from_right) + 2.0);
    }

    #[test]
    fn a_light_from_above_lights_the_forehead_more_than_the_chin() {
        let f = face(200.0, 200.0, 120.0);
        let mut above = light(f, 1.0, 0.5);
        above.probe = probe(0.0, -1.0, 0.0);
        let out = above.apply(dark(400, 400));
        let top = mean(&out, 1, 170, 230, 130, 175);
        let bottom = mean(&out, 1, 170, 230, 230, 275);
        assert!(top > bottom + 2.0, "{top} {bottom}");
        // Without a picture the light does not lean up or down.
        let flat = light(f, 1.0, 0.5).apply(dark(400, 400));
        let (t, b) = (
            mean(&flat, 1, 170, 230, 130, 175),
            mean(&flat, 1, 170, 230, 225, 270),
        );
        assert!((t - b).abs() < 6.0, "{t} {b}");
    }

    #[test]
    fn a_warm_light_picture_warms_the_light() {
        let f = face(200.0, 200.0, 120.0);
        let colour = |warmth: f32| {
            let mut light = light(f, 1.0, 0.5);
            light.probe = probe(-1.0, 0.0, warmth);
            let out = light.apply(dark(400, 400));
            mean(&out, 0, 150, 250, 150, 250) - mean(&out, 2, 150, 250, 150, 250)
        };
        assert!(colour(1.0) > colour(-1.0) + 3.0);
    }
}
