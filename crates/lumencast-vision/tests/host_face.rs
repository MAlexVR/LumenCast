//! Hardware check: finds the face of whoever sits in front of the camera.
//! Needs a person in view; nothing is saved. Ignored by default:
//!
//!   LUMENCAST_CAMERA=/dev/video2 cargo test -p lumencast-vision --test host_face \
//!       -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_core::cancel::CancelFlag;
use lumencast_core::mode::{CaptureMode, FrameInterval, FrameSize, PixelFormat};
use lumencast_core::session::{run_capture, SessionConfig};
use lumencast_fx::codec::decode_jpeg;
use lumencast_fx::reframe::FaceDetector;
use lumencast_v4l::device::{V4lCamera, V4lOptions};
use lumencast_vision::SeetaDetector;
use std::time::Instant;

#[test]
#[ignore = "needs a camera and a person in view"]
fn the_face_in_front_of_the_camera_is_found() {
    let path = std::env::var("LUMENCAST_CAMERA").expect("set LUMENCAST_CAMERA=/dev/videoN");
    let camera = V4lCamera::open_path(&path, V4lOptions::default()).unwrap();
    let mode = CaptureMode {
        format: PixelFormat::parse("MJPG").unwrap(),
        size: FrameSize::new(1280, 720).unwrap(),
        interval: FrameInterval::new(1, 30).unwrap(),
    };
    let mut frames = Vec::new();
    let config = SessionConfig {
        max_frames: Some(40),
        ..SessionConfig::default()
    };
    run_capture(&camera, &mode, config, &CancelFlag::new(), |frame| {
        frames.push(frame.data.clone());
        true
    })
    .unwrap();

    let mut detector = SeetaDetector::new().unwrap();
    let mut found = 0;
    let mut times = Vec::new();
    for jpeg in frames.iter().skip(10) {
        let frame = decode_jpeg(jpeg).unwrap();
        let start = Instant::now();
        let face = detector.detect(&frame);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        if let Some(face) = face {
            found += 1;
            println!(
                "face centre ({:.0}, {:.0}) height {:.0} in a {}x{} frame",
                face.center_x,
                face.center_y,
                face.height,
                frame.width(),
                frame.height()
            );
        }
    }
    times.sort_by(f64::total_cmp);
    println!(
        "detections: {found} of {}, median {:.1} ms, worst {:.1} ms",
        times.len(),
        times[times.len() / 2],
        times[times.len() - 1]
    );
    assert!(
        found >= times.len() / 2,
        "found a face in only {found} of {} frames",
        times.len()
    );
}
