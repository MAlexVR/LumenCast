//! Sends frames into the real v4l2loopback device and reads them back through
//! `v4l2-ctl`, the way another program would see the virtual camera. Needs the
//! `v4l2loopback` module loaded with a device and `v4l2-ctl` installed. Ignored
//! by default:
//!
//!   cargo test -p lumencast-v4l --test host_loopback -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_fx::convert::rgb_to_i420;
use lumencast_fx::frame::Frame;
use lumencast_v4l::output::{find_loopback, LoopbackOutput};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 360;

#[test]
#[ignore = "needs a v4l2loopback device and v4l2-ctl"]
fn frames_written_to_the_loopback_device_can_be_read_back() {
    let path = find_loopback().expect("no v4l2loopback device");
    let path = path.to_str().unwrap().to_owned();
    // Never fight another program for a device it is sending to (Blucast's
    // server, or LumenCast itself): the user would lose their virtual camera.
    let users = Command::new("fuser").arg(&path).output().unwrap();
    assert!(
        String::from_utf8_lossy(&users.stdout).trim().is_empty(),
        "{path} is in use; close the program that sends to it and run this again"
    );
    let mut output = LoopbackOutput::open(&path, WIDTH, HEIGHT).unwrap();
    assert_eq!(output.size(), (WIDTH, HEIGHT));

    // A picture with a bright left half and a dark right half.
    let frame = Frame::from_fn(WIDTH, HEIGHT, |x, _| {
        if x < WIDTH / 2 {
            [235; 3]
        } else {
            [20; 3]
        }
    });
    let (_, _, i420) = rgb_to_i420(&frame);
    let stop = Arc::new(AtomicBool::new(false));
    let writer_stop = Arc::clone(&stop);
    let writer = std::thread::spawn(move || {
        while !writer_stop.load(Ordering::SeqCst) {
            output.write(&i420).unwrap();
            std::thread::sleep(Duration::from_millis(33));
        }
    });

    std::thread::sleep(Duration::from_millis(500));
    let dir = std::env::temp_dir().join(format!("lumencast-loopback-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.yuv");
    let status = Command::new("v4l2-ctl")
        .args([
            "-d",
            &path,
            "--set-fmt-video",
            &format!("width={WIDTH},height={HEIGHT},pixelformat=YU12"),
            "--stream-mmap",
            "--stream-count=5",
            "--stream-to",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    stop.store(true, Ordering::SeqCst);
    writer.join().unwrap();
    assert!(
        status.status.success(),
        "v4l2-ctl failed: {}",
        String::from_utf8_lossy(&status.stderr)
    );
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let size = (WIDTH * HEIGHT * 3 / 2) as usize;
    assert!(bytes.len() >= size, "only {} bytes read", bytes.len());
    let luma = &bytes[bytes.len() - size..bytes.len() - size + (WIDTH * HEIGHT) as usize];
    let row = (HEIGHT / 2 * WIDTH) as usize;
    let (left, right) = (luma[row + 10], luma[row + WIDTH as usize - 10]);
    println!(
        "read back luma: left {left}, right {right} ({} bytes)",
        bytes.len()
    );
    assert!(
        left > 200 && right < 40,
        "the picture did not arrive intact"
    );
}
