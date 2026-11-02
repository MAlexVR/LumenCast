//! Compares two ways of cleaning low-light noise on synthetic sequences with a
//! known clean picture, so the numbers are exact: the recursive temporal filter
//! used by LumenCast and a spatial 3x3 edge-preserving filter (reference only).
//! Ignored by default because it prints a measurement:
//!
//!   cargo test -p lumencast-fx --release --test denoise_compare -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_fx::denoise::TemporalDenoise;
use lumencast_fx::frame::Frame;
use lumencast_fx::params::Strength;
use std::time::Instant;

const W: u32 = 640;
const H: u32 = 360;

struct Noise(u32);

impl Noise {
    fn next(&mut self, amplitude: i32) -> i32 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        ((self.0 >> 16) as i32 % (2 * amplitude + 1)) - amplitude
    }
}

/// A textured scene; a bright block moves 6 pixels per frame when `step` is set.
fn clean(x: u32, y: u32, block_at: Option<u32>) -> u8 {
    let texture = 70 + ((x / 6 + y / 6) % 2) * 50 + (x * 5 + y * 3) % 30;
    if let Some(left) = block_at {
        if x >= left && x < left + 80 && (100..220).contains(&y) {
            return 235;
        }
    }
    texture.min(255) as u8
}

fn clean_frame(block_at: Option<u32>) -> Frame {
    Frame::from_fn(W, H, |x, y| [clean(x, y, block_at); 3])
}

fn noisy_frame(noise: &mut Noise, block_at: Option<u32>, amplitude: i32) -> Frame {
    let mut data = Vec::with_capacity((W * H * 3) as usize);
    for y in 0..H {
        for x in 0..W {
            let base = i32::from(clean(x, y, block_at));
            for _ in 0..3 {
                data.push((base + noise.next(amplitude)).clamp(0, 255) as u8);
            }
        }
    }
    Frame::new(W, H, data).unwrap()
}

fn mse(a: &Frame, b: &Frame, region: Option<(u32, u32, u32, u32)>) -> f64 {
    let (x0, x1, y0, y1) = region.unwrap_or((0, W, 0, H));
    let (mut sum, mut count) = (0.0, 0.0);
    for y in y0..y1 {
        for x in x0..x1 {
            let (p, q) = (a.pixel(x, y), b.pixel(x, y));
            for c in 0..3 {
                sum += (f64::from(p[c]) - f64::from(q[c])).powi(2);
                count += 1.0;
            }
        }
    }
    sum / count
}

/// Reference: each channel becomes the average of the neighbours within 40
/// levels of it (a cheap bilateral filter).
fn spatial(frame: &Frame) -> Frame {
    let (w, h) = (frame.width() as usize, frame.height() as usize);
    let src = frame.data();
    let mut out = src.to_vec();
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            for c in 0..3 {
                let centre = i32::from(src[(y * w + x) * 3 + c]);
                let (mut sum, mut count) = (0, 0);
                for dy in 0..3 {
                    for dx in 0..3 {
                        let v = i32::from(src[((y + dy - 1) * w + x + dx - 1) * 3 + c]);
                        if (v - centre).abs() <= 40 {
                            sum += v;
                            count += 1;
                        }
                    }
                }
                out[(y * w + x) * 3 + c] = (sum / count) as u8;
            }
        }
    }
    Frame::new(frame.width(), frame.height(), out).unwrap()
}

#[test]
#[ignore = "prints a measurement; run with --release"]
fn compare_temporal_and_spatial_cleaning() {
    let strength = Strength::new(0.7).unwrap();
    // Static scene.
    let mut noise = Noise(5);
    let mut temporal = TemporalDenoise::new();
    let reference = clean_frame(None);
    let (mut raw, mut t_out, mut s_out) = (0.0, 0.0, 0.0);
    let (mut t_ms, mut s_ms) = (0.0, 0.0);
    for i in 0..40 {
        let frame = noisy_frame(&mut noise, None, 20);
        let start = Instant::now();
        let t = temporal.process(frame.clone(), strength);
        t_ms += start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let s = spatial(&frame);
        s_ms += start.elapsed().as_secs_f64() * 1000.0;
        if i == 39 {
            raw = mse(&frame, &reference, None);
            t_out = mse(&t, &reference, None);
            s_out = mse(&s, &reference, None);
        }
    }
    println!("static scene, noise +-20 (MSE against the clean picture, lower is better)");
    println!("  noisy input       {raw:>8.1}");
    println!(
        "  temporal (ours)   {t_out:>8.1}   {:.2} ms/frame",
        t_ms / 40.0
    );
    println!(
        "  spatial 3x3 ref   {s_out:>8.1}   {:.2} ms/frame",
        s_ms / 40.0
    );

    // Moving block: error measured only around the block's edges.
    let mut noise = Noise(9);
    let mut temporal = TemporalDenoise::new();
    let (mut t_edge, mut s_edge, mut raw_edge) = (0.0, 0.0, 0.0);
    for i in 0..30u32 {
        let left = 40 + i * 6;
        let frame = noisy_frame(&mut noise, Some(left), 20);
        let t = temporal.process(frame.clone(), strength);
        let s = spatial(&frame);
        if i == 29 {
            let truth = clean_frame(Some(left));
            let band = (left.saturating_sub(24), left + 104, 100, 220);
            raw_edge = mse(&frame, &truth, Some(band));
            t_edge = mse(&t, &truth, Some(band));
            s_edge = mse(&s, &truth, Some(band));
        }
    }
    println!("moving block (error around its edges)");
    println!("  noisy input       {raw_edge:>8.1}");
    println!("  temporal (ours)   {t_edge:>8.1}");
    println!("  spatial 3x3 ref   {s_edge:>8.1}");
    assert!(t_out < raw, "the temporal filter must clean a static scene");
    assert!(t_edge < raw_edge * 1.5, "and must not smear a moving edge");
}
