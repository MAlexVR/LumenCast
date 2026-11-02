//! How much the framing trembles over a real recording of a person sitting
//! still: the detector's box against the pan the framing ends up with, at the
//! rate the app offers frames (every third). Ignored by default:
//!
//!   LUMENCAST_FRAMES=/path/recording.mjpg cargo test -p lumencast-vision --release \
//!       --test host_jitter -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_fx::codec::decode_jpeg;
use lumencast_fx::params::Zoom;
use lumencast_fx::reframe::{FaceDetector, Reframer};
use lumencast_vision::SeetaDetector;

fn jpegs(bytes: &[u8]) -> Vec<&[u8]> {
    let starts: Vec<usize> = bytes
        .windows(3)
        .enumerate()
        .filter(|(_, w)| *w == [0xff, 0xd8, 0xff])
        .map(|(i, _)| i)
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, s)| &bytes[*s..*starts.get(n + 1).unwrap_or(&bytes.len())])
        .collect()
}

fn range(values: &[f32]) -> f32 {
    values.iter().cloned().fold(f32::MIN, f32::max)
        - values.iter().cloned().fold(f32::MAX, f32::min)
}

#[test]
#[ignore = "needs a recording with a person in view"]
fn the_framing_is_steadier_than_the_detector() {
    let bytes = std::fs::read(std::env::var("LUMENCAST_FRAMES").unwrap()).unwrap();
    let mut detector = SeetaDetector::new().unwrap();
    let mut reframer = Reframer::new(Zoom::new(1.3).unwrap(), 63);
    let (mut xs, mut ys, mut pans, mut pans_y) = (vec![], vec![], vec![], vec![]);
    for (n, jpeg) in jpegs(&bytes).into_iter().enumerate().skip(10) {
        if n % 3 != 0 {
            continue;
        }
        let frame = decode_jpeg(jpeg).unwrap();
        let Some(face) = detector.detect(&frame) else {
            continue;
        };
        xs.push(face.center_x);
        ys.push(face.center_y);
        let framing = reframer
            .observe(Some(face), frame.width(), frame.height(), 100_000)
            .unwrap();
        pans.push(framing.pan_x);
        pans_y.push(framing.pan_y);
    }
    let settled = pans.len() / 3;
    println!(
        "{} detections; face centre moved {:.0} px across and {:.0} px down; the pan moved {:.3} and {:.3} (1.0 is the whole margin)",
        xs.len(),
        range(&xs[settled..]),
        range(&ys[settled..]),
        range(&pans[settled..]),
        range(&pans_y[settled..])
    );
}
