//! Talks to the real matte server container on the real GPU. Needs the
//! container running (it is started by the app, or by hand) and saved camera
//! frames with a person in view; nothing is stored in the repository. Ignored
//! by default:
//!
//!   LUMENCAST_FRAMES=/path/frames.mjpg cargo test -p lumencast-nvidia \
//!       --release --test host_server -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_fx::background::{Backdrop, Background};
use lumencast_fx::codec::{decode_jpeg, encode_jpeg};
use lumencast_fx::stages::Stage;
use lumencast_nvidia::client::MatteClient;
use lumencast_nvidia::sidecar::{ensure_started, image_name, SystemPodman, PORT};
use std::time::{Duration, Instant};

fn last_jpeg(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .windows(3)
        .rposition(|w| w == [0xff, 0xd8, 0xff])
        .expect("no JPEG in the file");
    &bytes[start..]
}

#[test]
#[ignore = "needs the NVIDIA matte container and a GPU"]
fn the_nvidia_model_cuts_the_person_out_fast_enough() {
    ensure_started(&SystemPodman, &image_name()).unwrap();
    let frame = decode_jpeg(last_jpeg(
        &std::fs::read(std::env::var("LUMENCAST_FRAMES").unwrap()).unwrap(),
    ))
    .unwrap();
    let mut client = MatteClient::new(PORT);
    // The model takes a while to load the first time.
    let deadline = Instant::now() + Duration::from_secs(90);
    let mut matte = None;
    while matte.is_none() {
        matte = client.matte(&frame);
        assert!(Instant::now() < deadline, "the server never answered");
        std::thread::sleep(Duration::from_millis(250));
    }
    let mut times = Vec::new();
    for _ in 0..60 {
        let start = Instant::now();
        matte = client.matte(&frame);
        times.push(start.elapsed().as_secs_f64() * 1000.0);
        assert!(matte.is_some());
    }
    times.sort_by(f64::total_cmp);
    let matte = matte.unwrap();
    let person = matte.alpha().iter().filter(|a| **a > 127).count();
    println!(
        "nvidia matte {}x{}: {:.1} ms median, {:.1} ms worst, person covers {:.0}%",
        matte.width(),
        matte.height(),
        times[times.len() / 2],
        times[times.len() - 1],
        person as f64 / matte.alpha().len() as f64 * 100.0
    );
    if let Ok(out) = std::env::var("LUMENCAST_MATTE_OUT") {
        let composed = Background {
            matte,
            backdrop: Backdrop::Solid([0, 160, 0]),
        }
        .apply(frame);
        std::fs::write(out, encode_jpeg(&composed, 88).unwrap()).unwrap();
    }
}
