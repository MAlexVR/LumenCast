//! Runs the microphone chain over a real recording of a room (raw 48 kHz mono
//! 32-bit float samples, in `LUMENCAST_AUDIO`) and prints how much each effect
//! lowers it and how much of real time it uses. The recording is never kept in
//! the repository. Ignored by default:
//!
//!   LUMENCAST_AUDIO=/path/room.raw cargo test -p lumencast-audio --release \
//!       --test host_room -- --ignored --nocapture

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_audio::chain::{AudioChain, ChainSettings};
use lumencast_audio::meter::measure;
use lumencast_audio::voice::MicProfile;
use std::time::Instant;

fn samples() -> Vec<f32> {
    let bytes =
        std::fs::read(std::env::var("LUMENCAST_AUDIO").expect("set LUMENCAST_AUDIO")).unwrap();
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

fn run(settings: ChainSettings, input: &[f32]) -> (Vec<f32>, f64) {
    let mut chain = AudioChain::new(settings);
    let mut out = Vec::with_capacity(input.len());
    let start = Instant::now();
    for block in input.chunks(480) {
        out.extend(chain.process(block));
    }
    (out, start.elapsed().as_secs_f64())
}

#[test]
#[ignore = "needs a recording of a room"]
fn what_each_effect_does_to_the_noise_of_a_real_room() {
    let input = samples();
    let seconds = input.len() as f64 / 48_000.0;
    let settle = 24_000.min(input.len() / 2);
    let before = measure(&input[settle..]);
    println!(
        "recording: {seconds:.1} s, level {:.1} dB rms, peak {:.1} dB",
        before.rms_db, before.peak_db
    );
    for (name, settings) in [
        (
            "noise removal 100%",
            ChainSettings {
                noise_removal: Some(1.0),
                ..ChainSettings::default()
            },
        ),
        (
            "noise removal 50%",
            ChainSettings {
                noise_removal: Some(0.5),
                ..ChainSettings::default()
            },
        ),
        (
            "room echo removal 100%",
            ChainSettings {
                room_echo_removal: Some(1.0),
                ..ChainSettings::default()
            },
        ),
        (
            "studio voice",
            ChainSettings {
                studio_voice: Some(MicProfile::Headset),
                ..ChainSettings::default()
            },
        ),
        (
            "all three",
            ChainSettings {
                noise_removal: Some(1.0),
                room_echo_removal: Some(1.0),
                studio_voice: Some(MicProfile::Headset),
            },
        ),
    ] {
        let (out, took) = run(settings, &input);
        let after = measure(&out[settle..]);
        println!(
            "{name:<24} {:+.1} dB rms ({:.1} dB), uses {:.1}% of real time",
            after.rms_db - before.rms_db,
            after.rms_db,
            took / seconds * 100.0
        );
    }
}
