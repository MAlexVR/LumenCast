//! Starting, updating and stopping the two audio paths.

use crate::audio::io::{AudioIo, Capture, Closer, Playback};
use crate::audio::pump;
use crate::audio::settings::{AudioSettings, MicrophoneSettings, SpeakerSettings};
use crate::events::{AppEvent, EventSink};
use lumencast_audio::chain::ChainSettings;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The two paths audio takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AudioPath {
    Microphone,
    Speakers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum PathStatus {
    Off,
    Running,
    Unavailable { reason: String },
}

/// How often the levels are reported.
const LEVEL_INTERVAL: Duration = Duration::from_millis(100);

struct Running {
    /// The device it was started for; a different one means a restart.
    device: Option<String>,
    chain: Arc<Mutex<ChainSettings>>,
    closers: Vec<Closer>,
    stopping: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

#[derive(Default)]
struct Slot {
    running: Option<Running>,
    status: Option<PathStatus>,
}

struct State {
    microphone: Slot,
    speakers: Slot,
    reported: (PathStatus, PathStatus),
}

pub struct AudioEngine {
    sink: Arc<dyn EventSink>,
    io: Arc<dyn AudioIo>,
    state: Arc<Mutex<State>>,
}

fn status_of(slot: &Slot) -> PathStatus {
    slot.status.clone().unwrap_or(PathStatus::Off)
}

impl AudioEngine {
    pub fn new(sink: Arc<dyn EventSink>, io: Arc<dyn AudioIo>) -> Self {
        Self {
            sink,
            io,
            state: Arc::new(Mutex::new(State {
                microphone: Slot::default(),
                speakers: Slot::default(),
                reported: (PathStatus::Off, PathStatus::Off),
            })),
        }
    }

    /// Makes what runs match `settings`: starts what was switched on, stops
    /// what was switched off, restarts what changed device, and only updates
    /// the effects of what keeps running.
    pub fn apply(&self, settings: &AudioSettings) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        let microphone = settings.microphone.enabled.then(|| {
            (
                settings.microphone.source.clone(),
                settings.microphone.chain(),
            )
        });
        let speakers = settings
            .speakers
            .enabled
            .then(|| (settings.speakers.sink.clone(), settings.speakers.chain()));
        self.update(AudioPath::Microphone, &mut state.microphone, microphone);
        self.update(AudioPath::Speakers, &mut state.speakers, speakers);
        let now = (status_of(&state.microphone), status_of(&state.speakers));
        if now != state.reported {
            state.reported = now.clone();
            self.sink.emit(AppEvent::AudioStatus {
                microphone: now.0,
                speakers: now.1,
            });
        }
    }

    /// The status of both paths.
    pub fn status(&self) -> (PathStatus, PathStatus) {
        self.state
            .lock()
            .map(|state| (status_of(&state.microphone), status_of(&state.speakers)))
            .unwrap_or((PathStatus::Off, PathStatus::Off))
    }

    /// Stops everything.
    pub fn shutdown(&self) {
        self.apply(&AudioSettings {
            microphone: MicrophoneSettings::default(),
            speakers: SpeakerSettings::default(),
        });
    }

    fn update(
        &self,
        path: AudioPath,
        slot: &mut Slot,
        wanted: Option<(Option<String>, ChainSettings)>,
    ) {
        // Anything that ended by itself since the last call is cleaned up first.
        if slot
            .running
            .as_ref()
            .is_some_and(|r| r.handle.as_ref().is_some_and(JoinHandle::is_finished))
        {
            Self::stop(slot);
        }
        match wanted {
            None => {
                Self::stop(slot);
                slot.status = Some(PathStatus::Off);
            }
            Some((device, chain)) => {
                if let Some(running) = slot.running.as_ref().filter(|r| r.device == device) {
                    if let Ok(mut shared) = running.chain.lock() {
                        *shared = chain;
                    }
                    return;
                }
                Self::stop(slot);
                match self.start(path, device, chain) {
                    Ok(running) => {
                        slot.running = Some(running);
                        slot.status = Some(PathStatus::Running);
                    }
                    Err(reason) => slot.status = Some(PathStatus::Unavailable { reason }),
                }
            }
        }
    }

    fn stop(slot: &mut Slot) {
        if let Some(mut running) = slot.running.take() {
            running.stopping.store(true, Ordering::SeqCst);
            for close in &running.closers {
                close();
            }
            if let Some(handle) = running.handle.take() {
                let _ = handle.join();
            }
        }
    }

    fn start(
        &self,
        path: AudioPath,
        device: Option<String>,
        chain: ChainSettings,
    ) -> Result<Running, String> {
        let (mut input, mut output, channels): (Box<dyn Capture>, Box<dyn Playback>, usize) =
            match path {
                AudioPath::Microphone => (
                    self.io.open_source(device.as_deref())?,
                    self.io.open_virtual_microphone()?,
                    1,
                ),
                AudioPath::Speakers => (
                    self.io.open_virtual_speakers()?,
                    self.io.open_sink(device.as_deref())?,
                    2,
                ),
            };
        let closers = vec![input.closer()];
        let shared = Arc::new(Mutex::new(chain));
        let stopping = Arc::new(AtomicBool::new(false));
        let (thread_shared, thread_stopping) = (Arc::clone(&shared), Arc::clone(&stopping));
        let (sink, state) = (Arc::clone(&self.sink), Arc::clone(&self.state));
        let handle = std::thread::Builder::new()
            .name(format!("lumencast-audio-{path:?}").to_lowercase())
            .spawn(move || {
                let mut last_report: Option<Instant> = None;
                let result = pump::run(
                    &mut *input,
                    &mut *output,
                    channels,
                    &thread_shared,
                    &mut |heard, sent| {
                        if last_report.is_none_or(|at| at.elapsed() >= LEVEL_INTERVAL) {
                            last_report = Some(Instant::now());
                            sink.emit(AppEvent::AudioLevels {
                                path,
                                input_db: heard.rms_db,
                                output_db: sent.rms_db,
                            });
                        }
                    },
                );
                if thread_stopping.load(Ordering::SeqCst) {
                    return;
                }
                let reason = match result {
                    Ok(()) => "the audio stream ended".to_owned(),
                    Err(error) => format!("the audio stream failed: {error}"),
                };
                // It ended by itself: say so, so the UI does not show it as running.
                if let Ok(mut state) = state.lock() {
                    let slot = match path {
                        AudioPath::Microphone => &mut state.microphone,
                        AudioPath::Speakers => &mut state.speakers,
                    };
                    slot.status = Some(PathStatus::Unavailable { reason });
                    let now = (status_of(&state.microphone), status_of(&state.speakers));
                    state.reported = now.clone();
                    sink.emit(AppEvent::AudioStatus {
                        microphone: now.0,
                        speakers: now.1,
                    });
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Running {
            device,
            chain: shared,
            closers,
            stopping,
            handle: Some(handle),
        })
    }
}

impl Drop for AudioEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::settings::Effect;
    use crate::audio::testing::FakeIo;

    #[derive(Default)]
    struct Collected(Mutex<Vec<AppEvent>>);

    impl EventSink for Collected {
        fn emit(&self, event: AppEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    impl Collected {
        fn statuses(&self) -> Vec<(PathStatus, PathStatus)> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter_map(|e| match e {
                    AppEvent::AudioStatus {
                        microphone,
                        speakers,
                    } => Some((microphone.clone(), speakers.clone())),
                    _ => None,
                })
                .collect()
        }

        fn levels(&self) -> usize {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|e| matches!(e, AppEvent::AudioLevels { .. }))
                .count()
        }
    }

    fn engine() -> (AudioEngine, Arc<FakeIo>, Arc<Collected>) {
        let (io, sink) = (Arc::new(FakeIo::default()), Arc::new(Collected::default()));
        (AudioEngine::new(sink.clone(), io.clone()), io, sink)
    }

    /// Settings with the microphone on and every effect off, so audio passes unchanged.
    fn plain_microphone() -> AudioSettings {
        let mut settings = AudioSettings::default();
        settings.microphone.enabled = true;
        settings.microphone.noise_removal = Effect::new(false, 80);
        settings
    }

    fn wait_until(what: &str, condition: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn switching_the_microphone_on_publishes_it_and_the_audio_flows_through() {
        let (engine, io, sink) = engine();
        engine.apply(&plain_microphone());
        assert_eq!(
            *io.opened.lock().unwrap(),
            ["source:default", "virtual microphone"]
        );
        assert_eq!(engine.status().0, PathStatus::Running);
        let voice: Vec<f32> = (0..pump::BLOCK * 20)
            .map(|n| (n as f32 * 0.07).sin() * 0.2)
            .collect();
        io.input(0).feed(&voice);
        wait_until("the audio to come out", || {
            io.output_len(0) >= voice.len() * 4
        });
        wait_until("a level report", || sink.levels() > 0);
        engine.shutdown();
        assert_eq!(engine.status().0, PathStatus::Off);
        assert_eq!(
            sink.statuses(),
            vec![
                (PathStatus::Running, PathStatus::Off),
                (PathStatus::Off, PathStatus::Off)
            ]
        );
    }

    #[test]
    fn changing_an_effect_keeps_it_running_and_changing_the_device_restarts_it() {
        let (engine, io, _sink) = engine();
        let mut settings = plain_microphone();
        engine.apply(&settings);
        settings.microphone.studio_voice = true;
        settings.microphone.noise_removal = Effect::new(true, 50);
        engine.apply(&settings);
        assert_eq!(
            io.opened.lock().unwrap().len(),
            2,
            "no new streams for a change of effects"
        );
        settings.microphone.source = Some("alsa_input.other".into());
        engine.apply(&settings);
        assert_eq!(
            *io.opened.lock().unwrap(),
            [
                "source:default",
                "virtual microphone",
                "source:alsa_input.other",
                "virtual microphone"
            ]
        );
        assert_eq!(engine.status().0, PathStatus::Running);
    }

    #[test]
    fn a_device_that_cannot_be_opened_is_reported_and_tried_again_on_the_next_apply() {
        let (engine, io, sink) = engine();
        *io.broken.lock().unwrap() = Some("source:");
        engine.apply(&plain_microphone());
        assert!(
            matches!(engine.status().0, PathStatus::Unavailable { ref reason } if reason.contains("not there"))
        );
        *io.broken.lock().unwrap() = None;
        engine.apply(&plain_microphone());
        assert_eq!(engine.status().0, PathStatus::Running);
        assert!(sink.statuses().len() >= 2);
    }

    #[test]
    fn a_stream_that_ends_by_itself_is_reported_and_a_new_apply_starts_it_again() {
        let (engine, io, sink) = engine();
        engine.apply(&plain_microphone());
        io.input(0).end();
        wait_until("the end to be reported", || {
            matches!(engine.status().0, PathStatus::Unavailable { .. })
        });
        assert!(matches!(
            sink.statuses().last(),
            Some((PathStatus::Unavailable { .. }, _))
        ));
        engine.apply(&plain_microphone());
        assert_eq!(engine.status().0, PathStatus::Running);
        assert_eq!(
            io.opened
                .lock()
                .unwrap()
                .iter()
                .filter(|n| n.starts_with("source"))
                .count(),
            2
        );
    }

    #[test]
    fn the_speakers_take_what_plays_into_the_virtual_output_and_pass_it_to_the_real_one() {
        let (engine, io, _sink) = engine();
        let mut settings = AudioSettings::default();
        settings.speakers.enabled = true;
        settings.speakers.noise_removal = Effect::new(false, 70);
        settings.speakers.sink = Some("alsa_output.usb".into());
        engine.apply(&settings);
        assert_eq!(
            *io.opened.lock().unwrap(),
            ["virtual speakers", "sink:alsa_output.usb"]
        );
        let stereo: Vec<f32> = (0..pump::BLOCK * 2 * 4)
            .map(|n| (n as f32 * 0.03).sin() * 0.1)
            .collect();
        io.input(0).feed(&stereo);
        wait_until("the stereo audio to come out", || {
            io.output_len(0) >= stereo.len() * 4
        });
        assert_eq!(engine.status().1, PathStatus::Running);
        assert_eq!(
            engine.status().0,
            PathStatus::Off,
            "the microphone path is independent"
        );
    }

    #[test]
    fn dropping_the_engine_stops_everything() {
        let (engine, io, _sink) = engine();
        engine.apply(&plain_microphone());
        let input = io.input(0);
        drop(engine);
        assert!(
            input.closed.load(Ordering::SeqCst),
            "the input stream was closed"
        );
    }
}
