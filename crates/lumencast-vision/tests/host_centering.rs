//! Measures how well the auto framing centres a real face: detects the face in
//! a saved camera frame, frames it at the controller's target, detects it again
//! in the framed picture and reports how far it sits from the centre. Needs a
//! file of concatenated MJPEG frames with a person in view (nothing is stored
//! in the repository). Ignored by default:
//!
//!   LUMENCAST_FRAMES=/path/frames.mjpg cargo test -p lumencast-vision \
//!       --test host_centering -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_fx::codec::decode_jpeg;
use lumencast_fx::reframe::{target_framing, FaceDetector};
use lumencast_fx::stages::{ManualFraming, Stage};
use lumencast_vision::SeetaDetector;

fn last_jpeg(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .windows(3)
        .rposition(|w| w == [0xff, 0xd8, 0xff])
        .expect("no JPEG in the file");
    &bytes[start..]
}

#[test]
#[ignore = "needs saved camera frames with a person in view"]
fn the_framing_target_puts_the_face_at_the_centre() {
    let path = std::env::var("LUMENCAST_FRAMES").expect("set LUMENCAST_FRAMES=/path/frames.mjpg");
    let bytes = std::fs::read(path).unwrap();
    let frame = decode_jpeg(last_jpeg(&bytes)).unwrap();
    let mut detector = SeetaDetector::new().unwrap();
    let face = detector.detect(&frame).expect("no face in the frame");
    let (w, h) = (frame.width() as f32, frame.height() as f32);
    println!(
        "source: face centre ({:.0}, {:.0}) = ({:+.2}, {:+.2}) of the half size, height {:.0} ({:.0}% of the frame)",
        face.center_x,
        face.center_y,
        (face.center_x - w / 2.0) / (w / 2.0),
        (face.center_y - h / 2.0) / (h / 2.0),
        face.height,
        face.height / h * 100.0
    );
    for min_zoom in [1.15_f32, 1.26, 1.5] {
        let target = target_framing(face, frame.width(), frame.height(), min_zoom);
        let (zoom, pan_x, pan_y) = target.as_params();
        let framed = ManualFraming { zoom, pan_x, pan_y }.apply(frame.clone());
        let again = detector.detect(&framed);
        match again {
            Some(f) => println!(
                "min zoom {min_zoom}: target zoom {:.2} pan ({:+.2}, {:+.2}); face now at ({:+.2}, {:+.2}) of the half size",
                target.zoom,
                target.pan_x,
                target.pan_y,
                (f.center_x - w / 2.0) / (w / 2.0),
                (f.center_y - h / 2.0) / (h / 2.0)
            ),
            None => println!("min zoom {min_zoom}: face not found after framing"),
        }
    }
}
