//! How much the key light brightens a real face at several intensities: the
//! mean luma over the face and the difference between its two sides. Needs a
//! saved frame with a person in view. Ignored by default:
//!
//!   LUMENCAST_FRAMES=/path/frames.mjpg cargo test -p lumencast-vision --release \
//!       --test host_keylight -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_fx::codec::{decode_jpeg, encode_jpeg};
use lumencast_fx::frame::Frame;
use lumencast_fx::keylight::KeyLight;
use lumencast_fx::params::Strength;
use lumencast_fx::reframe::{FaceBox, FaceDetector};
use lumencast_fx::stages::Stage;
use lumencast_vision::SeetaDetector;

fn last_jpeg(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .windows(3)
        .rposition(|w| w == [0xff, 0xd8, 0xff])
        .expect("no JPEG in the file");
    &bytes[start..]
}

/// Mean luma of the left and right halves of the face area.
fn sides(frame: &Frame, face: FaceBox) -> (f32, f32) {
    let (mut left, mut right, mut nl, mut nr) = (0.0, 0.0, 0.0, 0.0);
    let r = face.height * 0.5;
    for y in (face.center_y - r) as u32..(face.center_y + r) as u32 {
        for x in (face.center_x - r) as u32..(face.center_x + r) as u32 {
            if x >= frame.width() || y >= frame.height() {
                continue;
            }
            let p = frame.pixel(x, y);
            let luma = 0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]);
            if (x as f32) < face.center_x {
                left += luma;
                nl += 1.0;
            } else {
                right += luma;
                nr += 1.0;
            }
        }
    }
    (left / nl, right / nr)
}

#[test]
#[ignore = "needs saved camera frames with a person in view"]
fn how_much_the_key_light_adds_to_a_real_face() {
    let bytes = std::fs::read(std::env::var("LUMENCAST_FRAMES").unwrap()).unwrap();
    let frame = decode_jpeg(last_jpeg(&bytes)).unwrap();
    let face = SeetaDetector::new()
        .unwrap()
        .detect(&frame)
        .expect("no face");
    let (l0, r0) = sides(&frame, face);
    println!("face luma without light: left {l0:.0}, right {r0:.0}");
    for intensity in [0.3, 0.6, 1.0] {
        let lit = KeyLight {
            face,
            intensity: Strength::new(intensity).unwrap(),
            warmth: Strength::new(0.45).unwrap(),
            probe: None,
        }
        .apply(frame.clone());
        let (l, r) = sides(&lit, face);
        println!(
            "intensity {:>3.0}%: left {l:.0} ({:+.0}), right {r:.0} ({:+.0}), side difference {:+.0}",
            intensity * 100.0,
            l - l0,
            r - r0,
            l - r
        );
        if let Ok(out) = std::env::var("LUMENCAST_LIGHT_OUT") {
            if (intensity - 0.6).abs() < 1e-3 {
                std::fs::write(out, encode_jpeg(&lit, 90).unwrap()).unwrap();
            }
        }
    }
}
