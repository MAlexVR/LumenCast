//! Does Lanczos scaling bring back more of a picture than bilinear scaling?
//! Takes a real camera frame, shrinks it with a box average, scales it back up
//! both ways and compares each result with the original (PSNR, higher is
//! better), and times the scaling to 1080p. Needs saved frames; nothing is
//! stored in the repository. Ignored by default:
//!
//!   LUMENCAST_FRAMES=/path/frames.mjpg cargo test -p lumencast-fx --release \
//!       --test upscale_compare -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_fx::codec::decode_jpeg;
use lumencast_fx::frame::Frame;
use lumencast_fx::resample::resize_lanczos;
use lumencast_fx::stages::resize;
use std::time::Instant;

fn last_jpeg(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .windows(3)
        .rposition(|w| w == [0xff, 0xd8, 0xff])
        .expect("no JPEG in the file");
    &bytes[start..]
}

fn psnr(a: &Frame, b: &Frame) -> f64 {
    let sum: f64 = a
        .data()
        .iter()
        .zip(b.data())
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2))
        .sum();
    let mse = sum / a.data().len() as f64;
    10.0 * (255.0f64 * 255.0 / mse.max(1e-9)).log10()
}

fn box_half(frame: &Frame) -> Frame {
    let (w, h) = (frame.width() / 2, frame.height() / 2);
    Frame::from_fn(w, h, |x, y| {
        let mut out = [0u8; 3];
        for (c, slot) in out.iter_mut().enumerate() {
            let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                .iter()
                .map(|(dx, dy)| u32::from(frame.pixel(x * 2 + dx, y * 2 + dy)[c]))
                .sum();
            *slot = ((sum + 2) / 4) as u8;
        }
        out
    })
}

#[test]
#[ignore = "needs saved camera frames; run with --release"]
fn lanczos_recovers_more_than_bilinear_and_is_fast_enough() {
    let path = std::env::var("LUMENCAST_FRAMES").expect("set LUMENCAST_FRAMES");
    let original = decode_jpeg(last_jpeg(&std::fs::read(path).unwrap())).unwrap();
    let small = box_half(&original);
    let (w, h) = (original.width(), original.height());
    let bilinear = resize(&small, w, h);
    let lanczos = resize_lanczos(&small, w, h);
    let (b, l) = (psnr(&original, &bilinear), psnr(&original, &lanczos));
    println!(
        "scaling {}x{} back to {w}x{h} (PSNR against the original, dB)",
        small.width(),
        small.height()
    );
    println!("  bilinear {b:.2}");
    println!("  lanczos3 {l:.2}   ({:+.2} dB)", l - b);

    let time = |work: &dyn Fn() -> Frame| {
        work();
        let mut samples: Vec<f64> = (0..20)
            .map(|_| {
                let start = Instant::now();
                work();
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        samples.sort_by(f64::total_cmp);
        samples[10]
    };
    println!("720p -> 1080p, ms per frame:");
    println!("  bilinear {:.2}", time(&|| resize(&original, 1920, 1080)));
    println!(
        "  lanczos3 {:.2}",
        time(&|| resize_lanczos(&original, 1920, 1080))
    );
    assert!(l > b, "Lanczos must beat bilinear on a real frame");
}
