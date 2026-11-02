//! Video noise reduction for low light. Our own design: a recursive temporal
//! filter that blends each pixel with the same pixel of the previous output,
//! and trusts the previous output less the more the pixel changed, so static
//! areas lose their noise and moving edges do not smear. NVIDIA Broadcast's
//! `video_noise_removal` runs a neural model with a temporal state per stream;
//! this keeps the same idea (state across frames) without a model.
//!
//! Unlike the other stages it has state, so it is a struct that is fed frames
//! in order instead of a `Stage`.

use crate::frame::Frame;
use crate::params::Strength;
use rayon::prelude::*;

/// Largest share of the previous output kept, at full strength and no motion.
const MAX_KEEP: f32 = 0.9;
/// Change (average over the three channels) that already counts as motion,
/// at strength 0 and at strength 1: noisier pictures need a wider tolerance.
const TOLERANCE_LOW: f32 = 10.0;
const TOLERANCE_HIGH: f32 = 40.0;

#[derive(Debug, Default)]
pub struct TemporalDenoise {
    previous: Option<Frame>,
}

impl TemporalDenoise {
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets the history, for example when the scene or the mode changes.
    pub fn reset(&mut self) {
        self.previous = None;
    }

    /// Cleans `frame` using the history and remembers the result.
    pub fn process(&mut self, mut frame: Frame, strength: Strength) -> Frame {
        let amount = strength.get();
        if amount <= 0.0 {
            self.previous = None;
            return frame;
        }
        let Some(previous) = self
            .previous
            .take()
            .filter(|p| (p.width(), p.height()) == (frame.width(), frame.height()))
        else {
            self.previous = Some(frame.clone());
            return frame;
        };
        let table = keep_table(amount);
        let before = previous.data();
        let (width, height) = (frame.width() as usize, frame.height() as usize);
        // Pixels the past could not help with (motion) are cleaned from their
        // neighbours instead; this records where.
        let mut moving = vec![false; width * height];
        frame
            .data_mut()
            .par_chunks_exact_mut(width * 3)
            .zip(before.par_chunks_exact(width * 3))
            .zip(moving.par_chunks_exact_mut(width))
            .for_each(|((now_row, then_row), moving_row)| {
                for (index, (now, then)) in now_row
                    .chunks_exact_mut(3)
                    .zip(then_row.chunks_exact(3))
                    .enumerate()
                {
                    let change = (i32::from(now[0]) - i32::from(then[0])).unsigned_abs()
                        + (i32::from(now[1]) - i32::from(then[1])).unsigned_abs()
                        + (i32::from(now[2]) - i32::from(then[2])).unsigned_abs();
                    let keep = i32::from(table[(change / 3).min(255) as usize]);
                    if keep < MOVING_BELOW {
                        moving_row[index] = true;
                    }
                    if keep == 0 {
                        continue;
                    }
                    for c in 0..3 {
                        let (a, b) = (i32::from(now[c]), i32::from(then[c]));
                        now[c] = (a + (((b - a) * keep) >> 8)) as u8;
                    }
                }
            });
        let frame = smooth_where(frame, &moving, amount);
        self.previous = Some(frame.clone());
        frame
    }
}

/// Keep weight (of 256) under which a pixel counts as moving.
const MOVING_BELOW: i32 = 60;
/// Neighbours this close to the centre (in levels) join the average.
const SPATIAL_RANGE: i32 = 40;

/// A 3x3 edge-preserving average, only on the pixels flagged in `moving` and
/// only as strongly as `amount` asks (a blend between the pixel and the average).
fn smooth_where(frame: Frame, moving: &[bool], amount: f32) -> Frame {
    if !moving.iter().any(|m| *m) {
        return frame;
    }
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    let source = frame.data().to_vec();
    let mut frame = frame;
    let blend = (amount * 256.0) as i32;
    frame
        .data_mut()
        .par_chunks_exact_mut(width * 3)
        .enumerate()
        .for_each(|(y, out_row)| {
            if y == 0 || y + 1 >= height {
                return;
            }
            for x in 1..width.saturating_sub(1) {
                if !moving[y * width + x] {
                    continue;
                }
                for c in 0..3 {
                    let centre = i32::from(source[(y * width + x) * 3 + c]);
                    let (mut sum, mut count) = (0, 0);
                    for dy in 0..3 {
                        let row = (y + dy - 1) * width + x - 1;
                        for dx in 0..3 {
                            let value = i32::from(source[(row + dx) * 3 + c]);
                            if (value - centre).abs() <= SPATIAL_RANGE {
                                sum += value;
                                count += 1;
                            }
                        }
                    }
                    let average = sum / count;
                    out_row[x * 3 + c] = (centre + (((average - centre) * blend) >> 8)) as u8;
                }
            }
        });
    frame
}

/// For each average change 0..=255, how much of the previous output to keep,
/// as a 0..=256 weight.
fn keep_table(amount: f32) -> [u16; 256] {
    let tolerance = TOLERANCE_LOW + (TOLERANCE_HIGH - TOLERANCE_LOW) * amount;
    let top = MAX_KEEP * amount;
    let mut table = [0u16; 256];
    for (change, slot) in table.iter_mut().enumerate() {
        let closeness = (1.0 - change as f32 / tolerance).clamp(0.0, 1.0);
        *slot = (top * closeness * 256.0).round() as u16;
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic noise in -amplitude..=amplitude.
    struct Noise(u32);

    impl Noise {
        fn next(&mut self, amplitude: i32) -> i32 {
            self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((self.0 >> 16) as i32 % (2 * amplitude + 1)) - amplitude
        }
    }

    fn clean(x: u32, y: u32) -> [u8; 3] {
        let v = (60 + (x * 3 + y * 2) % 120) as u8;
        [v, v, v]
    }

    fn noisy(noise: &mut Noise, amplitude: i32) -> Frame {
        let mut data = Vec::with_capacity(48 * 32 * 3);
        for y in 0..32 {
            for x in 0..48 {
                for c in clean(x, y) {
                    data.push((i32::from(c) + noise.next(amplitude)).clamp(0, 255) as u8);
                }
            }
        }
        Frame::new(48, 32, data).unwrap()
    }

    fn error(frame: &Frame) -> f64 {
        let mut sum = 0.0;
        for y in 0..frame.height() {
            for x in 0..frame.width() {
                let (a, b) = (frame.pixel(x, y), clean(x, y));
                for c in 0..3 {
                    sum += (f64::from(a[c]) - f64::from(b[c])).powi(2);
                }
            }
        }
        sum / f64::from(frame.width() * frame.height() * 3)
    }

    fn strength(value: f32) -> Strength {
        Strength::new(value).unwrap()
    }

    #[test]
    fn a_static_noisy_scene_gets_clearly_cleaner_over_time() {
        let mut noise = Noise(7);
        let mut filter = TemporalDenoise::new();
        let mut last = None;
        let mut raw_error = 0.0;
        for _ in 0..30 {
            let frame = noisy(&mut noise, 18);
            raw_error = error(&frame);
            last = Some(filter.process(frame, strength(0.8)));
        }
        let cleaned = error(&last.unwrap());
        assert!(cleaned < raw_error * 0.5, "{cleaned} vs {raw_error}");
    }

    #[test]
    fn the_first_frame_and_zero_strength_pass_through() {
        let mut noise = Noise(3);
        let mut filter = TemporalDenoise::new();
        let first = noisy(&mut noise, 10);
        assert_eq!(
            filter.process(first.clone(), strength(0.9)).data(),
            first.data()
        );
        let second = noisy(&mut noise, 10);
        assert_eq!(
            filter.process(second.clone(), strength(0.0)).data(),
            second.data()
        );
    }

    #[test]
    fn a_big_change_is_not_smeared_with_the_past() {
        let mut filter = TemporalDenoise::new();
        let dark = Frame::from_fn(16, 16, |_, _| [20; 3]);
        let bright = Frame::from_fn(16, 16, |_, _| [220; 3]);
        for _ in 0..5 {
            filter.process(dark.clone(), strength(1.0));
        }
        let out = filter.process(bright.clone(), strength(1.0));
        assert_eq!(out.data(), bright.data(), "a cut shows at once");
    }

    #[test]
    fn a_size_change_resets_the_history() {
        let mut filter = TemporalDenoise::new();
        filter.process(Frame::from_fn(16, 16, |_, _| [20; 3]), strength(1.0));
        let other = Frame::from_fn(8, 8, |_, _| [200; 3]);
        assert_eq!(
            filter.process(other.clone(), strength(1.0)).data(),
            other.data()
        );
    }

    #[test]
    fn a_stronger_setting_cleans_more() {
        let run = |amount: f32| {
            let mut noise = Noise(11);
            let mut filter = TemporalDenoise::new();
            let mut out = None;
            for _ in 0..20 {
                out = Some(filter.process(noisy(&mut noise, 18), strength(amount)));
            }
            error(&out.unwrap())
        };
        assert!(run(1.0) < run(0.3));
    }
}
