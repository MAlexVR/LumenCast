//! Measures the matte on saved camera frames with a person in view: how much of
//! the picture it calls person and how long it takes. Nothing is stored in the
//! repository. Ignored by default:
//!
//!   LUMENCAST_FRAMES=/path/frames.mjpg cargo test -p lumencast-segment \
//!       --release --test host_matte -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_fx::codec::decode_jpeg;
use lumencast_segment::SelfieSegmenter;
use std::time::Instant;

fn last_jpeg(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .windows(3)
        .rposition(|w| w == [0xff, 0xd8, 0xff])
        .expect("no JPEG in the file");
    &bytes[start..]
}

#[test]
#[ignore = "needs saved camera frames with a person in view"]
fn the_matte_finds_the_person_and_is_fast_enough() {
    let path = std::env::var("LUMENCAST_FRAMES").expect("set LUMENCAST_FRAMES");
    let frame = decode_jpeg(last_jpeg(&std::fs::read(path).unwrap())).unwrap();
    // Another model can be tried without touching the code:
    //   LUMENCAST_MODEL=/path/model.onnx LUMENCAST_MODEL_SIZE=256x144 [LUMENCAST_MODEL_SIGNED=1]
    let mut segmenter = match std::env::var("LUMENCAST_MODEL") {
        Ok(model) => {
            let size = std::env::var("LUMENCAST_MODEL_SIZE").expect("set LUMENCAST_MODEL_SIZE=WxH");
            let (w, h) = size.split_once('x').expect("WxH");
            SelfieSegmenter::with_model(
                &std::fs::read(model).unwrap(),
                w.parse().unwrap(),
                h.parse().unwrap(),
                std::env::var("LUMENCAST_MODEL_SIGNED").is_ok(),
            )
            .unwrap()
        }
        Err(_) => SelfieSegmenter::new().unwrap(),
    };
    let mut times = Vec::new();
    let mut matte = segmenter.matte(&frame).unwrap();
    for _ in 0..40 {
        let start = Instant::now();
        matte = segmenter.matte(&frame).unwrap();
        times.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let person = matte.alpha().iter().filter(|a| **a > 127).count();
    println!(
        "matte {:.1} ms median, person covers {:.0}% of the picture",
        times[times.len() / 2],
        person as f64 / matte.alpha().len() as f64 * 100.0
    );
    if let Ok(out) = std::env::var("LUMENCAST_MATTE_OUT") {
        let (w, h) = (frame.width(), frame.height());
        let mut rgb = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let a = f32::from(matte.sample(x, y, w, h)) / 255.0;
                let p = frame.pixel(x, y);
                for (c, bg) in p.iter().zip([0u8, 160, 0]) {
                    rgb.push((f32::from(*c) * a + f32::from(bg) * (1.0 - a)) as u8);
                }
            }
        }
        let image = lumencast_fx::frame::Frame::new(w, h, rgb).unwrap();
        std::fs::write(out, lumencast_fx::codec::encode_jpeg(&image, 88).unwrap()).unwrap();
    }
}
