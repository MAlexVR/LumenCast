//! The audio paths against the real PipeWire: the virtual microphone has to
//! show up as a source, carry the sound of a real microphone through the
//! effects to a program that reads it, and disappear when it is switched off;
//! the virtual speakers have to show up as an output. Nothing is played out
//! loud. The "real" microphone is a stand-in source made here, so the test does
//! not depend on what a room sounds like. Needs PipeWire with `pw-cat`,
//! `pw-loopback` and `pw-dump`. Ignored by default:
//!
//!   cargo test -p lumencast-app --test host_audio -- --ignored --nocapture --test-threads=1

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_app::audio::io::PipeWireIo;
use lumencast_app::audio::{AudioEngine, AudioSettings, PathStatus};
use lumencast_app::events::{AppEvent, EventSink};
use lumencast_audio::meter::measure;
use std::io::Write;
use std::process::Child;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Collector(Mutex<Vec<AppEvent>>);

impl EventSink for Collector {
    fn emit(&self, event: AppEvent) {
        self.0.lock().unwrap().push(event);
    }
}

/// A source that plays a 440 Hz tone, standing in for a microphone.
struct ToneSource {
    children: Vec<Child>,
}

impl ToneSource {
    const NAME: &'static str = "lumencast_test_source";

    fn start() -> Self {
        let loopback = Command::new("pw-loopback")
            .args([
                "-n",
                Self::NAME,
                "-i",
                "{ media.class=Audio/Sink node.name=lumencast_test_source_input }",
                "-o",
                "{ media.class=Audio/Source node.name=lumencast_test_source }",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        wait_for("the stand-in source to appear", || {
            nodes().contains("lumencast_test_source_input")
        });
        std::thread::sleep(Duration::from_millis(500));
        let mut player = Command::new("pw-cat")
            .args([
                "--playback",
                "--target",
                "lumencast_test_source_input",
                "--rate",
                "48000",
                "--channels",
                "2",
                "--format",
                "f32",
                "--raw",
                "-",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut stdin = player.stdin.take().unwrap();
        std::thread::spawn(move || {
            let bytes: Vec<u8> = (0..48_000 * 12)
                .flat_map(|n| {
                    let v = 0.2 * (2.0 * std::f32::consts::PI * 440.0 * n as f32 / 48_000.0).sin();
                    [v, v]
                })
                .flat_map(f32::to_le_bytes)
                .collect();
            let _ = stdin.write_all(&bytes);
        });
        Self {
            children: vec![player, loopback],
        }
    }
}

impl Drop for ToneSource {
    fn drop(&mut self) {
        for child in &mut self.children {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn nodes() -> String {
    String::from_utf8_lossy(&Command::new("pw-dump").output().unwrap().stdout).into_owned()
}

fn wait_for(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "needs PipeWire and a microphone"]
fn the_virtual_microphone_carries_the_real_one_through_the_effects_and_goes_away() {
    let tone = ToneSource::start();
    let sink = Arc::new(Collector::default());
    let engine = AudioEngine::new(sink.clone(), Arc::new(PipeWireIo));
    let mut settings = AudioSettings::default();
    settings.microphone.enabled = true;
    settings.microphone.source = Some(ToneSource::NAME.to_owned());
    settings.microphone.noise_removal.enabled = false;
    settings.microphone.studio_voice = true;
    engine.apply(&settings);
    assert_eq!(
        engine.status().0,
        PathStatus::Running,
        "{:?}",
        engine.status()
    );
    wait_for("the virtual microphone to appear", || {
        nodes().contains("lumencast_microphone")
    });

    // Read what another program would hear from it.
    let read = Command::new("pw-cat")
        .args([
            "--record",
            "--target",
            "lumencast_microphone",
            "--rate",
            "48000",
            "--channels",
            "1",
            "--format",
            "f32",
            "--raw",
            "-n",
            "96000",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .unwrap();
    let samples: Vec<f32> = read
        .stdout
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    assert!(
        samples.len() >= 48_000,
        "only {} samples were read",
        samples.len()
    );
    let heard = measure(&samples[24_000..]);
    println!(
        "the virtual microphone carries {:.1} dB rms ({} samples)",
        heard.rms_db,
        samples.len()
    );
    let reported = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, AppEvent::AudioLevels { .. }))
        .count();
    println!("{reported} level reports were sent");

    assert!(reported > 5);
    assert!(heard.rms_db > -95.0, "the virtual microphone is silent");

    settings.microphone.enabled = false;
    engine.apply(&settings);
    wait_for("the virtual microphone to go away", || {
        !nodes().contains("lumencast_microphone")
    });
    assert_eq!(engine.status().0, PathStatus::Off);
    drop(tone);
}

#[test]
#[ignore = "needs PipeWire"]
fn the_virtual_speakers_show_up_as_an_output_and_go_away() {
    let sink = Arc::new(Collector::default());
    let engine = AudioEngine::new(sink, Arc::new(PipeWireIo));
    let mut settings = AudioSettings::default();
    settings.speakers.enabled = true;
    engine.apply(&settings);
    assert_eq!(
        engine.status().1,
        PathStatus::Running,
        "{:?}",
        engine.status()
    );
    wait_for("the virtual speakers to appear", || {
        let dump = nodes();
        dump.contains("lumencast_speakers") && dump.contains("Audio/Sink")
    });
    // Silence into it: the path works end to end and nothing is heard.
    let mut player = Command::new("pw-cat")
        .args([
            "--playback",
            "--target",
            "lumencast_speakers",
            "--rate",
            "48000",
            "--channels",
            "2",
            "--format",
            "f32",
            "--raw",
            "-",
        ])
        .stdin(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = player.stdin.take().unwrap();
    stdin.write_all(&vec![0u8; 48_000 * 2 * 4]).unwrap();
    drop(stdin);
    std::thread::sleep(Duration::from_millis(500));
    let _ = player.kill();
    let _ = player.wait();
    assert_eq!(engine.status().1, PathStatus::Running);
    engine.shutdown();
    wait_for("the virtual speakers to go away", || {
        !nodes().contains("lumencast_speakers")
    });
}
