//! Host-device acceptance tests. They need a real camera and are ignored by
//! default, so `cargo test` stays offline and hardware-free:
//!
//!   LUMENCAST_CAMERA=/dev/video0 cargo test -p lumencast-v4l --test host_acceptance \
//!       -- --ignored --nocapture
//!
//! Optional: `LUMENCAST_MODE=MJPG:1280x720:30` (default) and
//! `LUMENCAST_SAVE_DIR=<existing dir>` to write a few raw frames for visual
//! comparison with the live preview. Nothing here changes system devices.

#![allow(clippy::unwrap_used, clippy::expect_used)] // test-only helpers

use lumencast_core::cancel::CancelFlag;
use lumencast_core::mode::{CaptureMode, FrameInterval, FrameSize, PixelFormat};
use lumencast_core::session::{run_capture, CameraPort, EndReason, SessionConfig};
use lumencast_v4l::device::{list_cameras, V4lCamera, V4lOptions};
use std::fs;
use std::path::PathBuf;

fn camera_path() -> String {
    std::env::var("LUMENCAST_CAMERA").expect("set LUMENCAST_CAMERA=/dev/videoN to the capture node")
}

/// `FOURCC:WIDTHxHEIGHT:FPS`, for example `MJPG:1280x720:30`.
fn requested_mode() -> CaptureMode {
    let text = std::env::var("LUMENCAST_MODE").unwrap_or_else(|_| "MJPG:1280x720:30".into());
    let parts: Vec<&str> = text.split(':').collect();
    let size: Vec<u32> = parts[1]
        .split('x')
        .map(|part| part.parse().unwrap())
        .collect();
    CaptureMode {
        format: PixelFormat::parse(parts[0]).unwrap(),
        size: FrameSize::new(size[0], size[1]).unwrap(),
        interval: FrameInterval::new(1, parts[2].parse().unwrap()).unwrap(),
    }
}

#[test]
#[ignore = "needs a real camera"]
fn lists_capture_nodes() {
    let cameras = list_cameras();
    println!("{}", serde_json::to_string_pretty(&cameras).unwrap());
    assert!(!cameras.is_empty(), "no V4L2 capture node found");
}

#[test]
#[ignore = "needs a real camera"]
fn probe_reports_only_modes_the_driver_advertised() {
    let camera = V4lCamera::open_path(&camera_path(), V4lOptions::default()).unwrap();
    let probe = camera.probe().unwrap();
    for mode in probe.capabilities.modes() {
        println!("supported: {mode}");
    }
    println!(
        "skipped: {}",
        serde_json::to_string_pretty(&probe.skipped).unwrap()
    );
    assert!(!probe.capabilities.modes().is_empty());
    for mode in probe.capabilities.modes() {
        assert_eq!(probe.capabilities.validate(mode).as_ref(), Ok(mode));
    }
}

#[test]
#[ignore = "needs a real camera"]
fn captures_the_requested_mode_without_loading_any_model() {
    let camera = V4lCamera::open_path(&camera_path(), V4lOptions::default()).unwrap();
    let request = requested_mode();
    let save_dir = std::env::var("LUMENCAST_SAVE_DIR").ok().map(PathBuf::from);
    let mut saved = 0;
    let mut seen = 0u64;
    let config = SessionConfig {
        max_frames: Some(120),
        ..SessionConfig::default()
    };
    let outcome = run_capture(&camera, &request, config, &CancelFlag::new(), |frame| {
        seen += 1;
        if let Some(dir) = &save_dir {
            if seen % 15 == 1 && saved < 8 {
                let name = dir.join(format!("frame-{saved:02}-{}us.jpg", frame.timestamp_us));
                if fs::write(name, &frame.data).is_ok() {
                    saved += 1;
                }
            }
        }
        true
    })
    .unwrap();

    println!("{}", serde_json::to_string_pretty(&outcome).unwrap());
    assert_eq!(
        outcome.end,
        EndReason::FrameLimit,
        "capture did not complete cleanly"
    );
    assert_eq!(outcome.stats.frames_captured, 120);
    let measured = outcome.stats.measured_fps.expect("measured fps");
    assert!(
        measured > 0.5 * request.interval.fps(),
        "measured {measured} fps"
    );
    // Informational: a driver that adjusts the mode is reported, not failed.
    println!("exact negotiation: {}", outcome.report.is_exact());
}

#[test]
#[ignore = "needs a real camera"]
fn rejects_an_unadvertised_mode_before_opening_the_stream() {
    let camera = V4lCamera::open_path(&camera_path(), V4lOptions::default()).unwrap();
    let impossible = CaptureMode {
        format: PixelFormat::parse("MJPG").unwrap(),
        size: FrameSize::new(7680, 4320).unwrap(),
        interval: FrameInterval::new(1, 240).unwrap(),
    };
    let error = camera
        .capabilities()
        .unwrap()
        .validate(&impossible)
        .unwrap_err();
    assert!(error.to_string().contains("not supported"));
}

/// Regression: a camera whose first frame takes longer than the poll interval
/// (the integrated one, while it adjusts exposure) used to fail with EINVAL.
/// Needs no `LUMENCAST_CAMERA`: it walks every capture node.
#[test]
#[ignore = "needs real cameras"]
fn every_listed_camera_delivers_frames_at_a_basic_mjpeg_mode() {
    let cameras = list_cameras();
    assert!(!cameras.is_empty(), "no V4L2 capture node found");
    let basic = CaptureMode {
        format: PixelFormat::parse("MJPG").unwrap(),
        size: FrameSize::new(640, 480).unwrap(),
        interval: FrameInterval::new(1, 30).unwrap(),
    };
    for info in cameras {
        let camera = V4lCamera::open_path(&info.path, V4lOptions::default()).unwrap();
        if camera.capabilities().unwrap().validate(&basic).is_err() {
            println!("{}: no MJPG 640x480 @ 30, skipped", info.path);
            continue;
        }
        let config = SessionConfig {
            max_frames: Some(30),
            ..SessionConfig::default()
        };
        let outcome = run_capture(&camera, &basic, config, &CancelFlag::new(), |_| true).unwrap();
        println!(
            "{} ({:?}): {:?}, {} frames, {:?} fps",
            info.path,
            info.name,
            outcome.end,
            outcome.stats.frames_captured,
            outcome.stats.measured_fps
        );
        assert_eq!(outcome.end, EndReason::FrameLimit, "{}", info.path);
        assert_eq!(outcome.stats.frames_captured, 30, "{}", info.path);
    }
}
