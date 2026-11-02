//! A light picture for the key light, our version of NVIDIA Broadcast's HDRI
//! option: the studio light does not come from a fixed lamp but from wherever a
//! picture is brightest, with the colour it has there. NVIDIA relights with a
//! neural model fed by an environment map; this reads the same kind of picture
//! and takes the three things the visible effect depends on: which side the
//! light comes from, how directional it is, and whether it is warm or cool.

use crate::frame::Frame;

/// Pixels sampled per side at most; the picture is only read, never shown.
const SAMPLES: usize = 96;
/// The brightest share of the picture that counts as "the light".
const BRIGHT_SHARE: f32 = 0.08;
/// Pixels below this fraction of the brightest one are not part of the light.
const BRIGHTEST_FRACTION: f32 = 0.7;
/// Contrast between the light and the rest of the picture that makes the
/// light fully directional.
const FULL_CONTRAST: f32 = 150.0;
/// The least directional a picture can be, so even a flat one leans a little.
const MIN_FOCUS: f32 = 0.2;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightProbe {
    /// Where the light comes from across the picture: -1 the left edge, 1 the right.
    pub x: f32,
    /// Where it comes from up and down: -1 the top, 1 the bottom.
    pub y: f32,
    /// -1 a cool light, 1 a warm one.
    pub warmth: f32,
    /// 0 a light from everywhere, 1 a light from one place.
    pub focus: f32,
}

fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

impl LightProbe {
    /// Reads the light out of `picture`; `None` for a picture with no light in
    /// it (all black).
    pub fn from_picture(picture: &Frame) -> Option<Self> {
        let (width, height) = (picture.width() as usize, picture.height() as usize);
        if width == 0 || height == 0 {
            return None;
        }
        let (step_x, step_y) = ((width / SAMPLES).max(1), (height / SAMPLES).max(1));
        let mut samples: Vec<(f32, f32, [f32; 3])> = Vec::new();
        for y in (0..height).step_by(step_y) {
            for x in (0..width).step_by(step_x) {
                let p = picture.pixel(x as u32, y as u32);
                samples.push((
                    (x as f32 + 0.5) / width as f32,
                    (y as f32 + 0.5) / height as f32,
                    [f32::from(p[0]), f32::from(p[1]), f32::from(p[2])],
                ));
            }
        }
        let mean = samples.iter().map(|s| luma(s.2)).sum::<f32>() / samples.len() as f32;
        let mut lumas: Vec<f32> = samples.iter().map(|s| luma(s.2)).collect();
        lumas.sort_by(|a, b| b.total_cmp(a));
        let count = ((samples.len() as f32 * BRIGHT_SHARE) as usize).max(1);
        // The brightest share of the picture, but never pixels much dimmer than
        // its brightest point: a small lamp in a dark room is the lamp, not the room.
        let threshold = lumas[count - 1].max(lumas[0] * BRIGHTEST_FRACTION);
        if threshold < 8.0 {
            return None;
        }
        let (mut sx, mut sy, mut total) = (0.0f32, 0.0f32, 0.0f32);
        let mut colour = [0.0f32; 3];
        let mut bright_mean = 0.0f32;
        let mut used = 0.0f32;
        for (x, y, c) in samples.iter().filter(|s| luma(s.2) >= threshold) {
            // The brighter the pixel, the more it pulls the light towards itself.
            let weight = luma(*c).max(1.0);
            sx += x * weight;
            sy += y * weight;
            total += weight;
            for k in 0..3 {
                colour[k] += c[k];
            }
            bright_mean += luma(*c);
            used += 1.0;
        }
        let (x, y) = (sx / total, sy / total);
        let bright_mean = bright_mean / used;
        let warmth = ((colour[0] - colour[2]) / used / 90.0).clamp(-1.0, 1.0);
        let focus = ((bright_mean - mean) / FULL_CONTRAST).clamp(MIN_FOCUS, 1.0);
        Some(Self {
            x: ((x - 0.5) * 2.0).clamp(-1.0, 1.0),
            y: ((y - 0.5) * 2.0).clamp(-1.0, 1.0),
            warmth,
            focus,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A dark picture with a bright patch of `colour` centred at `at` (0..1).
    fn picture(at: (f32, f32), colour: [u8; 3]) -> Frame {
        Frame::from_fn(200, 100, |x, y| {
            let (px, py) = (x as f32 / 200.0, y as f32 / 100.0);
            if (px - at.0).abs() < 0.08 && (py - at.1).abs() < 0.12 {
                colour
            } else {
                [20, 20, 20]
            }
        })
    }

    #[test]
    fn the_light_comes_from_where_the_picture_is_brightest() {
        let left = LightProbe::from_picture(&picture((0.1, 0.5), [255, 255, 255])).unwrap();
        assert!(left.x < -0.7 && left.y.abs() < 0.2, "{left:?}");
        let top_right = LightProbe::from_picture(&picture((0.9, 0.15), [255, 255, 255])).unwrap();
        assert!(top_right.x > 0.7 && top_right.y < -0.5, "{top_right:?}");
    }

    #[test]
    fn a_warm_light_reads_warm_and_a_cool_one_cool() {
        let warm = LightProbe::from_picture(&picture((0.5, 0.5), [255, 190, 120])).unwrap();
        let cool = LightProbe::from_picture(&picture((0.5, 0.5), [140, 190, 255])).unwrap();
        assert!(warm.warmth > 0.5 && cool.warmth < -0.5, "{warm:?} {cool:?}");
    }

    #[test]
    fn a_small_bright_light_is_directional_and_a_flat_picture_is_not() {
        let spot = LightProbe::from_picture(&picture((0.2, 0.5), [255, 255, 255])).unwrap();
        assert!(spot.focus > 0.8, "{spot:?}");
        let flat = LightProbe::from_picture(&Frame::from_fn(100, 100, |_, _| [128; 3])).unwrap();
        assert!((flat.focus - MIN_FOCUS).abs() < 1e-3, "{flat:?}");
    }

    #[test]
    fn a_black_picture_has_no_light() {
        assert!(LightProbe::from_picture(&Frame::from_fn(50, 50, |_, _| [0; 3])).is_none());
    }
}
