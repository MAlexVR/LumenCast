//! `AppCore`: the testable heart behind the Tauri commands.

use crate::audio::devices::{list_devices, AudioDevice};
use crate::audio::io::{AudioIo, PipeWireIo};
use crate::audio::{AudioEngine, AudioSettings, PathStatus};
use crate::error::AppError;
use crate::events::{AppEvent, EventSink};
use crate::preview::{needs_every_frame, FxRenderer, PreviewRenderer, PreviewWorker};
use crate::processing::ProcessingSettings;
use crate::settings::{
    config_dir, import_legacy, FileStore, ImportReport, LegacyFile, LegacySource, NoStore,
    PersistedSettings, Preferences, PreferredCamera, SettingsStore,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use lumencast_core::camera::{CameraCatalog, CameraInfo, CameraProbe, SkippedMode};
use lumencast_core::cancel::CancelFlag;
use lumencast_core::error::CaptureError;
use lumencast_core::mode::{CameraCapabilities, CaptureMode, NegotiationReport};
use lumencast_core::session::{RawFrame, SessionConfig, SessionObserver};
use lumencast_core::stats::StatsSnapshot;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{self, JoinHandle};

#[derive(Debug, Clone, Copy)]
pub struct AppConfig {
    /// Minimum capture-time spacing between preview frames.
    pub preview_interval_us: u64,
    /// Minimum capture-time spacing between stats events.
    pub stats_interval_us: u64,
    /// Frames the preview worker may hold; when full, the preview skips frames
    /// instead of slowing the capture.
    pub preview_queue_depth: usize,
    pub session: SessionConfig,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            preview_interval_us: 30_000,
            stats_interval_us: 500_000,
            preview_queue_depth: 1,
            session: SessionConfig::default(),
        }
    }
}

/// Frames the preview worker may hold for a stateful stream such as H264.
const STATEFUL_QUEUE_DEPTH: usize = 8;
/// A little slack so one slow frame does not drop the next ones of the virtual camera.
const DELIVERY_QUEUE_DEPTH: usize = 3;

/// Where settings are kept and where legacy ones may be imported from.
pub struct Storage {
    pub store: Arc<dyn SettingsStore>,
    pub legacy: Option<Arc<dyn LegacySource>>,
    /// The user's home, to translate the container paths of the legacy file.
    pub home: Option<String>,
}

impl Storage {
    /// LumenCast's file next to the legacy Blucast one, both under the user's
    /// configuration directory. Nothing is stored when there is no home.
    pub fn standard(xdg_config_home: Option<&str>, home: Option<&str>) -> Self {
        let Some(config) = config_dir(xdg_config_home, home) else {
            return Self::none();
        };
        Self {
            store: Arc::new(FileStore::new(config.join("lumencast/settings.json"))),
            legacy: Some(Arc::new(LegacyFile::new(
                config.join("blucast/settings.json"),
            ))),
            home: home.map(str::to_owned),
        }
    }

    pub fn none() -> Self {
        Self {
            store: Arc::new(NoStore),
            legacy: None,
            home: None,
        }
    }
}

pub struct AppCore {
    shared: Arc<Shared>,
}

struct Shared {
    catalog: Arc<dyn CameraCatalog>,
    sink: Arc<dyn EventSink>,
    config: AppConfig,
    renderer: Arc<dyn PreviewRenderer>,
    storage: Storage,
    audio: AudioEngine,
    /// Everything that is saved; `processing` below mirrors its processing part.
    saved: Mutex<PersistedSettings>,
    startup: Mutex<Startup>,
    processing: Mutex<ProcessingSettings>,
    /// Whether anyone is looking at the preview: false while the window is
    /// hidden in the tray, when nothing is encoded for it.
    preview_visible: AtomicBool,
    active: Mutex<Option<Active>>,
    next_id: AtomicU64,
}

/// What happened while loading settings, reported to the UI.
struct Startup {
    import_report: Option<ImportReport>,
    problem: Option<String>,
}

struct Active {
    id: u64,
    cancel: CancelFlag,
    handle: JoinHandle<()>,
}

impl AppCore {
    pub fn new(
        catalog: Arc<dyn CameraCatalog>,
        sink: Arc<dyn EventSink>,
        config: AppConfig,
    ) -> Self {
        Self::build(
            catalog,
            sink,
            config,
            Arc::new(FxRenderer::seeta()),
            Storage::none(),
            Arc::new(PipeWireIo),
        )
    }

    /// Like `with_renderer`, with persistent settings and the legacy source.
    pub fn with_storage(
        catalog: Arc<dyn CameraCatalog>,
        sink: Arc<dyn EventSink>,
        config: AppConfig,
        storage: Storage,
    ) -> Self {
        Self::build(
            catalog,
            sink,
            config,
            Arc::new(FxRenderer::seeta()),
            storage,
            Arc::new(PipeWireIo),
        )
    }

    /// Like `with_storage`, with other audio devices (tests use fake ones).
    pub fn with_audio_io(
        catalog: Arc<dyn CameraCatalog>,
        sink: Arc<dyn EventSink>,
        config: AppConfig,
        storage: Storage,
        audio: Arc<dyn AudioIo>,
    ) -> Self {
        Self::build(
            catalog,
            sink,
            config,
            Arc::new(FxRenderer::seeta()),
            storage,
            audio,
        )
    }

    /// Like `new`, with another preview renderer (tests use a slow one).
    pub fn with_renderer(
        catalog: Arc<dyn CameraCatalog>,
        sink: Arc<dyn EventSink>,
        config: AppConfig,
        renderer: Arc<dyn PreviewRenderer>,
    ) -> Self {
        Self::build(
            catalog,
            sink,
            config,
            renderer,
            Storage::none(),
            Arc::new(PipeWireIo),
        )
    }

    fn build(
        catalog: Arc<dyn CameraCatalog>,
        sink: Arc<dyn EventSink>,
        config: AppConfig,
        renderer: Arc<dyn PreviewRenderer>,
        storage: Storage,
        audio_io: Arc<dyn AudioIo>,
    ) -> Self {
        let (saved, import_report, problem) = load_settings(catalog.as_ref(), &storage);
        let audio = AudioEngine::new(Arc::clone(&sink), audio_io);
        // What was switched on last time starts again with the program.
        audio.apply(&saved.audio);
        Self {
            shared: Arc::new(Shared {
                catalog,
                sink,
                config,
                renderer,
                processing: Mutex::new(saved.processing.clone()),
                saved: Mutex::new(saved),
                startup: Mutex::new(Startup {
                    import_report,
                    problem,
                }),
                storage,
                audio,
                active: Mutex::new(None),
                next_id: AtomicU64::new(1),
                preview_visible: AtomicBool::new(true),
            }),
        }
    }

    pub fn list_cameras(&self) -> Vec<CameraInfo> {
        self.shared.catalog.list()
    }

    /// Cameras are addressed by index; the device path never comes from the UI.
    /// Modes the preview cannot show (for example H264) are moved to `skipped`
    /// with the reason, so the UI never offers a capture nobody could see.
    pub fn probe(&self, index: usize) -> Result<CameraProbe, AppError> {
        let path = self.device_path(index)?;
        Ok(only_previewable(
            self.shared.catalog.probe(&path)?,
            self.shared.renderer.as_ref(),
        ))
    }

    /// Starts a capture on its own thread; progress arrives as events.
    pub fn start(&self, index: usize, request: CaptureMode) -> Result<(), AppError> {
        let camera = self.camera(index)?;
        let path = camera.path.clone();
        let fourcc = request.format.to_string();
        if self.shared.renderer.supports(&fourcc).is_err() {
            return Err(AppError::FormatNotPreviewable(fourcc));
        }
        let mut active = self.shared.lock_active();
        if active.is_some() {
            return Err(AppError::AlreadyRunning);
        }
        self.remember(&camera, request);
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let cancel = CancelFlag::new();
        let shared = Arc::clone(&self.shared);
        let thread_cancel = cancel.clone();
        let handle = thread::Builder::new()
            .name("lumencast-capture".into())
            .spawn(move || shared.run_session(id, &path, &request, &thread_cancel))
            .map_err(|error| {
                CaptureError::Acquisition(format!("cannot start capture thread: {error}"))
            })?;
        *active = Some(Active { id, cancel, handle });
        Ok(())
    }

    /// Cancels the running capture and waits until the camera is released.
    pub fn stop(&self) -> Result<(), AppError> {
        let active = self
            .shared
            .lock_active()
            .take()
            .ok_or(AppError::NotRunning)?;
        active.cancel.cancel();
        active
            .handle
            .join()
            .map_err(|_| CaptureError::Acquisition("capture thread panicked".into()))?;
        Ok(())
    }

    /// Ends everything the program started: the capture, and the virtual audio
    /// devices, which are separate processes that would otherwise outlive it.
    pub fn shutdown(&self) {
        let _ = self.stop();
        self.shared.audio.shutdown();
    }

    /// Settings for the UI at startup; the import report is returned once.
    pub fn preferences(&self) -> Preferences {
        let mut startup = self.shared.lock_startup();
        Preferences {
            settings: self.shared.lock_saved().clone(),
            import_report: startup.import_report.take(),
            problem: startup.problem.clone(),
        }
    }

    pub fn set_language(&self, code: &str) -> Result<(), AppError> {
        if !matches!(code, "es" | "en") {
            return Err(AppError::InvalidSetting(format!(
                "language must be \"es\" or \"en\", got {code:?}"
            )));
        }
        self.shared.lock_saved().language = Some(code.to_owned());
        self.shared.persist().map_err(AppError::SettingsNotSaved)
    }

    /// Tells the program whether its window is showing; while it is not, no
    /// preview is produced (the virtual camera and the audio carry on).
    pub fn set_preview_visible(&self, visible: bool) {
        self.shared
            .preview_visible
            .store(visible, Ordering::Relaxed);
    }

    /// Whether closing the window keeps the program in the system tray.
    pub fn close_to_tray(&self) -> bool {
        self.shared.lock_saved().close_to_tray
    }

    pub fn set_close_to_tray(&self, enabled: bool) -> Result<(), AppError> {
        self.shared.lock_saved().close_to_tray = enabled;
        self.shared.persist().map_err(AppError::SettingsNotSaved)
    }

    /// Whether the program looks for a new version when it starts.
    pub fn check_updates(&self) -> bool {
        self.shared.lock_saved().check_updates
    }

    pub fn set_check_updates(&self, enabled: bool) -> Result<(), AppError> {
        self.shared.lock_saved().check_updates = enabled;
        self.shared.persist().map_err(AppError::SettingsNotSaved)
    }

    /// Validates and stores the settings; they apply to the next preview frame,
    /// also while a capture is running.
    pub fn set_processing(&self, settings: ProcessingSettings) -> Result<(), AppError> {
        settings.validate()?;
        *self.shared.lock_processing() = settings.clone();
        self.shared.lock_saved().processing = settings;
        // The setting is already live; only saving it can fail.
        self.shared.persist().map_err(AppError::SettingsNotSaved)
    }

    /// The audio settings in use.
    pub fn audio_settings(&self) -> AudioSettings {
        self.shared.lock_saved().audio.clone()
    }

    /// Validates, applies (starting, updating or stopping the audio paths) and
    /// saves the audio settings.
    pub fn set_audio(&self, settings: AudioSettings) -> Result<(), AppError> {
        settings.validate()?;
        self.shared.audio.apply(&settings);
        self.shared.lock_saved().audio = settings;
        self.shared.persist().map_err(AppError::SettingsNotSaved)
    }

    /// The status of the microphone and speaker paths.
    pub fn audio_status(&self) -> (PathStatus, PathStatus) {
        self.shared.audio.status()
    }

    /// The microphones and outputs the system has.
    pub fn audio_devices(&self) -> Result<Vec<AudioDevice>, AppError> {
        list_devices().map_err(AppError::AudioUnavailable)
    }

    pub fn processing(&self) -> ProcessingSettings {
        self.shared.lock_processing().clone()
    }

    pub fn is_running(&self) -> bool {
        self.shared.lock_active().is_some()
    }

    fn camera(&self, index: usize) -> Result<CameraInfo, AppError> {
        self.shared
            .catalog
            .list()
            .into_iter()
            .find(|camera| camera.index == index)
            .ok_or(AppError::UnknownCamera(index))
    }

    fn device_path(&self, index: usize) -> Result<String, AppError> {
        self.camera(index).map(|camera| camera.path)
    }

    /// Remembers the camera and the exact mode of a capture the user started.
    /// A failure to save must not stop the capture, but it is not silent.
    fn remember(&self, camera: &CameraInfo, mode: CaptureMode) {
        {
            let mut saved = self.shared.lock_saved();
            saved.preferred_camera = Some(PreferredCamera {
                usb_id: camera.usb_id.clone(),
                name: camera.name.clone(),
            });
            saved.preferred_mode = Some(mode);
        }
        if let Err(error) = self.shared.persist() {
            eprintln!("lumencast: could not save the last camera and mode: {error}");
        }
    }
}

impl Shared {
    fn lock_processing(&self) -> MutexGuard<'_, ProcessingSettings> {
        self.processing
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_saved(&self) -> MutexGuard<'_, PersistedSettings> {
        self.saved.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_startup(&self) -> MutexGuard<'_, Startup> {
        self.startup.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn persist(&self) -> Result<(), String> {
        let text =
            serde_json::to_string_pretty(&*self.lock_saved()).map_err(|error| error.to_string())?;
        self.storage.store.save(&text)
    }

    fn lock_active(&self) -> MutexGuard<'_, Option<Active>> {
        self.active.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn run_session(&self, id: u64, path: &str, request: &CaptureMode, cancel: &CancelFlag) {
        let mut observer = EventObserver {
            shared: self,
            mode: None,
            worker: None,
            last_preview_us: None,
            last_stats_us: None,
        };
        let result = self
            .catalog
            .run(path, request, self.config.session, cancel, &mut observer);
        // Everything queued for the preview is announced before the capture ends.
        observer.finish();
        // Free the slot before announcing the end, so the UI may start again at once.
        let mut active = self.lock_active();
        if active.as_ref().is_some_and(|current| current.id == id) {
            *active = None;
        }
        drop(active);
        self.sink.emit(match result {
            Ok(outcome) => AppEvent::Ended {
                outcome: Some(outcome),
                error: None,
            },
            Err(error) => AppEvent::Ended {
                outcome: None,
                error: Some(error.payload()),
            },
        });
    }
}

struct EventObserver<'a> {
    shared: &'a Shared,
    /// The mode the camera granted, known once the stream is open.
    mode: Option<CaptureMode>,
    /// Started on the first frame that needs processing.
    worker: Option<PreviewWorker>,
    last_preview_us: Option<u64>,
    last_stats_us: Option<u64>,
}

/// Loads the saved settings, or imports the legacy file the first time, or
/// falls back to defaults with a problem the UI can show.
fn load_settings(
    catalog: &dyn CameraCatalog,
    storage: &Storage,
) -> (PersistedSettings, Option<ImportReport>, Option<String>) {
    let fallback = |problem: String| (PersistedSettings::default(), None, Some(problem));
    match storage.store.load() {
        Err(error) => fallback(format!("could not read the saved settings: {error}")),
        Ok(Some(text)) => match serde_json::from_str::<PersistedSettings>(&text) {
            Err(error) => fallback(format!("the saved settings are not valid: {error}")),
            Ok(settings) => match settings.processing.validate() {
                Ok(()) => (settings, None, None),
                Err(error) => fallback(format!("the saved settings are not valid: {error}")),
            },
        },
        Ok(None) => {
            let Some(legacy) = storage.legacy.as_ref().and_then(|source| source.read()) else {
                return (PersistedSettings::default(), None, None);
            };
            match import_legacy(&legacy, storage.home.as_deref(), &catalog.list()) {
                Err(error) => fallback(format!("could not import the Blucast settings: {error}")),
                Ok(imported) => {
                    let saved = serde_json::to_string_pretty(&imported.settings)
                        .map_err(|error| error.to_string())
                        .and_then(|text| storage.store.save(&text));
                    let problem = saved.err().map(|error| {
                        format!("imported the Blucast settings but could not save them: {error}")
                    });
                    (imported.settings, Some(imported.report), problem)
                }
            }
        }
    }
}

fn is_due(last: Option<u64>, now: u64, interval: u64) -> bool {
    last.is_none_or(|last| now.saturating_sub(last) >= interval)
}

/// Moves the modes the preview cannot show into `skipped`, one explanation
/// per format and size, with the renderer's reason.
fn only_previewable(probe: CameraProbe, renderer: &dyn PreviewRenderer) -> CameraProbe {
    let (shown, hidden): (Vec<CaptureMode>, Vec<CaptureMode>) = probe
        .capabilities
        .modes()
        .iter()
        .copied()
        .partition(|mode| renderer.supports(&mode.format.to_string()).is_ok());
    let mut skipped = probe.skipped;
    let mut explained = std::collections::BTreeSet::new();
    for mode in hidden {
        let format = mode.format.to_string();
        let size = (mode.size.width, mode.size.height);
        if explained.insert((format.clone(), size)) {
            let reason = renderer
                .supports(&format)
                .err()
                .unwrap_or_else(|| format!("no preview decoder for {format} yet"));
            skipped.push(SkippedMode {
                reason,
                format,
                size: Some(size),
            });
        }
    }
    CameraProbe {
        capabilities: CameraCapabilities::new(shown),
        skipped,
    }
}

impl EventObserver<'_> {
    fn finish(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            worker.finish();
        }
    }
}

impl SessionObserver for EventObserver<'_> {
    fn negotiated(&mut self, report: &NegotiationReport) {
        self.mode = Some(report.negotiated);
        self.shared.sink.emit(AppEvent::Negotiated {
            report: report.clone(),
        });
    }

    fn frame(&mut self, frame: &RawFrame, stats: &StatsSnapshot) -> bool {
        let now = frame.timestamp_us;
        let config = self.shared.config;
        if is_due(self.last_stats_us, now, config.stats_interval_us) {
            self.last_stats_us = Some(now);
            self.shared.sink.emit(AppEvent::Stats {
                stats: stats.clone(),
            });
        }
        let Some(mode) = self.mode else {
            return false;
        };
        let settings = self.shared.lock_processing().clone();
        let stateful = needs_every_frame(&mode.format.to_string());
        // The virtual camera needs every frame processed, not only the ones the preview shows.
        let deliver = settings.virtual_camera.enabled;
        // Nothing is encoded for a preview nobody can see (the window is in the tray).
        let due = self.shared.preview_visible.load(Ordering::Relaxed)
            && is_due(self.last_preview_us, now, config.preview_interval_us);
        if !due && !stateful && !deliver {
            return false;
        }
        if due {
            // An attempt counts for the throttle even when it is skipped or fails,
            // so a slow or broken stream never makes the preview retry on every frame.
            self.last_preview_us = Some(now);
        }
        if due && settings.is_identity() && mode.format.bytes() == *b"MJPG" {
            // Nothing to change: the camera's own JPEG goes straight to the UI.
            self.shared.sink.emit(AppEvent::Frame {
                timestamp_us: now,
                jpeg_base64: BASE64.encode(&frame.data),
            });
            return true;
        }
        // A stateful stream needs every frame, so it gets a deeper queue.
        let depth = if stateful {
            config.preview_queue_depth.max(STATEFUL_QUEUE_DEPTH)
        } else if deliver {
            config.preview_queue_depth.max(DELIVERY_QUEUE_DEPTH)
        } else {
            config.preview_queue_depth
        };
        let worker = self.worker.get_or_insert_with(|| {
            PreviewWorker::spawn(
                Arc::clone(&self.shared.sink),
                Arc::clone(&self.shared.renderer),
                depth,
            )
        });
        let queued = worker.submit(now, &frame.data, settings, mode, due, deliver);
        if stateful && !queued {
            // The decoder missed a frame: the picture may show artefacts until
            // the next keyframe, and the user is told.
            worker.note_skipped();
        }
        due && queued
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::AppEvent;
    use lumencast_core::cancel::CancelFlag;
    use lumencast_core::error::CaptureError;
    use lumencast_core::mode::{CameraCapabilities, FrameInterval, FrameSize, PixelFormat};
    use lumencast_core::session::{
        run_capture_observed, CameraPort, EndReason, FrameSource, RawFrame, SessionObserver,
        SessionOutcome,
    };
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    fn mode(den: u32) -> CaptureMode {
        CaptureMode {
            format: PixelFormat::parse("MJPG").unwrap(),
            size: FrameSize::new(1280, 720).unwrap(),
            interval: FrameInterval::new(1, den).unwrap(),
        }
    }

    fn mode_of(fourcc: &str, width: u32, height: u32, den: u32) -> CaptureMode {
        CaptureMode {
            format: PixelFormat::parse(fourcc).unwrap(),
            size: FrameSize::new(width, height).unwrap(),
            interval: FrameInterval::new(1, den).unwrap(),
        }
    }

    struct FakeSource {
        negotiated: CaptureMode,
        produced: u64,
        frames: u64,
        endless: bool,
        repeat: bool,
        payload: Vec<u8>,
    }

    impl FrameSource for FakeSource {
        fn negotiated(&self) -> CaptureMode {
            self.negotiated
        }
        fn next_frame(&mut self) -> Result<Option<RawFrame>, CaptureError> {
            if self.produced < self.frames || self.repeat {
                if self.repeat {
                    std::thread::sleep(Duration::from_millis(1));
                }
                let frame = RawFrame {
                    timestamp_us: self.produced * 33_333,
                    data: self.payload.clone(),
                };
                self.produced += 1;
                return Ok(Some(frame));
            }
            if self.endless {
                std::thread::sleep(Duration::from_millis(1));
                return Ok(None);
            }
            Err(CaptureError::Acquisition("script exhausted".into()))
        }
    }

    struct FakeCamera {
        frames: u64,
        endless: bool,
        repeat: bool,
        payload: Vec<u8>,
        mode: CaptureMode,
    }

    impl CameraPort for FakeCamera {
        fn capabilities(&self) -> Result<CameraCapabilities, CaptureError> {
            Ok(CameraCapabilities::new([self.mode]))
        }
        fn open(&self, request: &CaptureMode) -> Result<Box<dyn FrameSource + '_>, CaptureError> {
            Ok(Box::new(FakeSource {
                negotiated: *request,
                produced: 0,
                frames: self.frames,
                endless: self.endless,
                repeat: self.repeat,
                payload: self.payload.clone(),
            }))
        }
    }

    struct FakeCatalog {
        frames: u64,
        endless: bool,
        repeat: bool,
        payload: Vec<u8>,
        mode: CaptureMode,
    }

    impl CameraCatalog for FakeCatalog {
        fn list(&self) -> Vec<CameraInfo> {
            vec![CameraInfo {
                path: "/dev/video2".into(),
                index: 2,
                name: Some("Fake".into()),
                usb_id: Some("04e8:205d".into()),
            }]
        }
        fn probe(&self, _device_path: &str) -> Result<CameraProbe, CaptureError> {
            Ok(CameraProbe {
                capabilities: CameraCapabilities::new([
                    self.mode,
                    mode_of("H264", 1280, 720, 30),
                    mode_of("H264", 1280, 720, 20),
                    mode_of("YUYV", 640, 480, 30),
                ]),
                skipped: vec![],
            })
        }
        fn run(
            &self,
            _device_path: &str,
            request: &CaptureMode,
            config: SessionConfig,
            cancel: &CancelFlag,
            observer: &mut dyn SessionObserver,
        ) -> Result<SessionOutcome, CaptureError> {
            let camera = FakeCamera {
                frames: self.frames,
                endless: self.endless,
                repeat: self.repeat,
                payload: self.payload.clone(),
                mode: self.mode,
            };
            run_capture_observed(&camera, request, config, cancel, observer)
        }
    }

    #[derive(Default)]
    struct Collector(Mutex<Vec<AppEvent>>);

    impl EventSink for Collector {
        fn emit(&self, event: AppEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    impl Collector {
        fn events(&self) -> Vec<AppEvent> {
            self.0.lock().unwrap().clone()
        }
        fn wait_for(&self, what: &str, done: impl Fn(&[AppEvent]) -> bool) -> Vec<AppEvent> {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let events = self.events();
                if done(&events) {
                    return events;
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {what}: {events:?}"
                );
                std::thread::sleep(Duration::from_millis(2));
            }
        }
        fn ended(events: &[AppEvent]) -> bool {
            events
                .iter()
                .any(|event| matches!(event, AppEvent::Ended { .. }))
        }
    }

    fn app(frames: u64, endless: bool) -> (AppCore, Arc<Collector>) {
        let sink = Arc::new(Collector::default());
        let config = AppConfig {
            preview_interval_us: 60_000,
            stats_interval_us: 100_000,
            preview_queue_depth: 16,
            session: SessionConfig {
                max_frames: Some(frames),
                ..SessionConfig::default()
            },
        };
        let catalog = Arc::new(FakeCatalog {
            frames,
            endless,
            repeat: false,
            payload: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mode: mode(30),
        });
        (AppCore::new(catalog, sink.clone(), config), sink)
    }

    /// A catalog whose camera sends `payload` as every frame.
    fn app_sending(payload: Vec<u8>, frames: u64, repeat: bool) -> (AppCore, Arc<Collector>) {
        app_sending_in(mode(30), payload, frames, repeat)
    }

    fn app_sending_in(
        camera_mode: CaptureMode,
        payload: Vec<u8>,
        frames: u64,
        repeat: bool,
    ) -> (AppCore, Arc<Collector>) {
        let sink = Arc::new(Collector::default());
        let config = AppConfig {
            preview_interval_us: 60_000,
            stats_interval_us: 100_000,
            preview_queue_depth: 16,
            session: SessionConfig {
                max_frames: (!repeat).then_some(frames),
                ..SessionConfig::default()
            },
        };
        let catalog = Arc::new(FakeCatalog {
            frames,
            endless: repeat,
            repeat,
            payload,
            mode: camera_mode,
        });
        (AppCore::new(catalog, sink.clone(), config), sink)
    }

    fn endless_app() -> (AppCore, Arc<Collector>) {
        let sink = Arc::new(Collector::default());
        let catalog = Arc::new(FakeCatalog {
            frames: 0,
            endless: true,
            repeat: false,
            payload: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mode: mode(30),
        });
        (
            AppCore::new(catalog, sink.clone(), AppConfig::default()),
            sink,
        )
    }

    #[test]
    fn lists_the_catalog_cameras() {
        let (core, _sink) = app(1, false);
        assert_eq!(core.list_cameras()[0].index, 2);
    }

    #[test]
    fn probe_and_start_address_cameras_by_index_only() {
        let (core, sink) = app(1, false);
        assert_eq!(core.probe(9).unwrap_err().code(), "device_not_found");
        assert_eq!(
            core.start(9, mode(30)).unwrap_err().code(),
            "device_not_found"
        );
        assert!(core.probe(2).is_ok());
        assert!(sink.events().is_empty());
        assert!(!core.is_running());
    }

    #[test]
    fn a_capture_reports_negotiation_first_and_the_outcome_last() {
        let (core, sink) = app(6, false);
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        assert!(
            matches!(events.first(), Some(AppEvent::Negotiated { report }) if report.is_exact())
        );
        match events.last() {
            Some(AppEvent::Ended {
                outcome: Some(outcome),
                error: None,
            }) => {
                assert_eq!(outcome.end, EndReason::FrameLimit);
                assert_eq!(outcome.stats.frames_captured, 6);
            }
            other => panic!("unexpected last event {other:?}"),
        }
    }

    #[test]
    fn preview_frames_are_throttled_by_capture_time_and_counted() {
        let (core, sink) = app(6, false);
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        let previews = events
            .iter()
            .filter(|event| matches!(event, AppEvent::Frame { .. }))
            .count();
        assert_eq!(previews, 3);
        match events.last() {
            Some(AppEvent::Ended {
                outcome: Some(outcome),
                ..
            }) => {
                assert_eq!(outcome.stats.frames_previewed, 3);
            }
            other => panic!("unexpected last event {other:?}"),
        }
    }

    #[test]
    fn frame_events_carry_the_jpeg_as_base64_with_its_capture_time() {
        let (core, sink) = app(1, false);
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        let json = events
            .iter()
            .find(|event| matches!(event, AppEvent::Frame { .. }))
            .map(|event| serde_json::to_value(event).unwrap())
            .unwrap();
        assert_eq!(json["kind"], "frame");
        assert_eq!(json["jpegBase64"], "/9j/2Q==");
        assert_eq!(json["timestampUs"], 0);
    }

    #[test]
    fn stats_events_are_rate_limited_and_start_with_the_first_frame() {
        let (core, sink) = app(6, false);
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        let stats: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AppEvent::Stats { stats } => Some(stats.frames_captured),
                _ => None,
            })
            .collect();
        assert_eq!(stats, [1, 5]);
    }

    #[test]
    fn unsupported_modes_end_with_an_error_and_free_the_camera() {
        let (core, sink) = app(3, false);
        core.start(2, mode(60)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        match events.last() {
            Some(AppEvent::Ended {
                outcome: None,
                error: Some(error),
            }) => {
                assert_eq!(error.code, "mode_unsupported");
            }
            other => panic!("unexpected last event {other:?}"),
        }
        assert!(!core.is_running());
    }

    #[test]
    fn only_one_capture_runs_at_a_time_and_stop_cancels_it() {
        let (core, sink) = endless_app();
        core.start(2, mode(30)).unwrap();
        assert!(core.is_running());
        assert_eq!(
            core.start(2, mode(30)).unwrap_err().code(),
            "capture_already_running"
        );

        core.stop().unwrap();
        assert!(!core.is_running());
        let events = sink.wait_for("end", Collector::ended);
        match events.last() {
            Some(AppEvent::Ended {
                outcome: Some(outcome),
                ..
            }) => {
                assert_eq!(outcome.end, EndReason::Cancelled);
            }
            other => panic!("unexpected last event {other:?}"),
        }
        core.start(2, mode(30)).unwrap();
        core.stop().unwrap();
    }

    #[test]
    fn stopping_without_a_capture_is_a_typed_error() {
        let (core, _sink) = app(1, false);
        assert_eq!(core.stop().unwrap_err().code(), "capture_not_running");
    }

    #[test]
    fn negotiated_event_serializes_with_a_kind_tag() {
        let (core, sink) = app(1, false);
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        let json = serde_json::to_value(&events[0]).unwrap();
        assert_eq!(json["kind"], "negotiated");
        assert_eq!(json["report"]["differences"], serde_json::json!([]));
    }

    /// 64x32 picture: black left half, white right half, as a real JPEG.
    fn halves_jpeg() -> Vec<u8> {
        let frame = lumencast_fx::frame::Frame::from_fn(64, 32, |x, _| {
            if x < 32 {
                [0, 0, 0]
            } else {
                [255, 255, 255]
            }
        });
        lumencast_fx::codec::encode_jpeg(&frame, 92).unwrap()
    }

    fn previews(events: &[AppEvent]) -> Vec<lumencast_fx::frame::Frame> {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        events
            .iter()
            .filter_map(|event| match event {
                AppEvent::Frame { jpeg_base64, .. } => Some(
                    lumencast_fx::codec::decode_jpeg(&STANDARD.decode(jpeg_base64).unwrap())
                        .unwrap(),
                ),
                _ => None,
            })
            .collect()
    }

    fn settings(zoom: f32, pan_x: f32, mirror: bool) -> ProcessingSettings {
        ProcessingSettings {
            zoom_percent: zoom,
            pan_x,
            mirror_preview: mirror,
            ..ProcessingSettings::default()
        }
    }

    #[test]
    fn settings_are_validated_stored_and_readable() {
        let (core, _sink) = app(1, false);
        assert_eq!(core.processing(), ProcessingSettings::default());
        core.set_processing(settings(150.0, 0.25, true)).unwrap();
        assert_eq!(core.processing(), settings(150.0, 0.25, true));
        let error = core
            .set_processing(settings(250.0, 0.0, false))
            .unwrap_err();
        assert_eq!(error.code(), "invalid_setting");
        assert_eq!(core.processing(), settings(150.0, 0.25, true));
    }

    #[test]
    fn neutral_settings_pass_the_camera_jpeg_through_untouched() {
        use base64::engine::general_purpose::STANDARD;
        use base64::Engine;
        let payload = halves_jpeg();
        let (core, sink) = app_sending(payload.clone(), 3, false);
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        let first = events
            .iter()
            .find_map(|event| match event {
                AppEvent::Frame { jpeg_base64, .. } => Some(jpeg_base64.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(first, STANDARD.encode(payload));
    }

    #[test]
    fn a_hidden_window_gets_no_preview_but_the_capture_goes_on() {
        let (core, sink) = app_sending(halves_jpeg(), 6, false);
        core.set_processing(settings(100.0, 0.0, true)).unwrap();
        core.set_preview_visible(false);
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        assert!(
            previews(&events).is_empty(),
            "nothing is encoded for nobody"
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, AppEvent::Stats { .. } | AppEvent::Ended { .. })));
    }

    #[test]
    fn the_mirror_flips_the_preview_frames() {
        let (core, sink) = app_sending(halves_jpeg(), 3, false);
        core.set_processing(settings(100.0, 0.0, true)).unwrap();
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        let shown = previews(&events);
        assert!(!shown.is_empty());
        for frame in shown {
            assert!(frame.pixel(4, 16)[0] > 200, "left should now be white");
            assert!(frame.pixel(59, 16)[0] < 55, "right should now be black");
        }
    }

    #[test]
    fn zoom_and_pan_crop_the_preview() {
        let (core, sink) = app_sending(halves_jpeg(), 3, false);
        core.set_processing(settings(200.0, 1.0, false)).unwrap();
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        let shown = previews(&events);
        assert!(!shown.is_empty());
        for frame in shown {
            assert_eq!((frame.width(), frame.height()), (64, 32));
            assert!((0..64).all(|x| frame.pixel(x, 16)[0] > 200));
        }
    }

    #[test]
    fn an_undecodable_preview_frame_is_dropped_and_counted_without_stopping() {
        let (core, sink) = app_sending(vec![1, 2, 3, 4, 5, 6, 7, 8], 6, false);
        core.set_processing(settings(100.0, 0.0, true)).unwrap();
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        assert!(previews(&events).is_empty());
        let dropped: Vec<u64> = events
            .iter()
            .filter_map(|event| match event {
                AppEvent::PreviewDropped { total } => Some(*total),
                _ => None,
            })
            .collect();
        assert_eq!(dropped, [1, 2, 3]);
        match events.last() {
            Some(AppEvent::Ended {
                outcome: Some(outcome),
                ..
            }) => {
                assert_eq!(outcome.end, EndReason::FrameLimit);
                assert_eq!(outcome.stats.frames_captured, 6);
                // Three frames were handed to the preview; each one then failed
                // to decode and was announced as dropped.
                assert_eq!(outcome.stats.frames_previewed, 3);
            }
            other => panic!("unexpected last event {other:?}"),
        }
    }

    #[test]
    fn settings_changed_during_a_capture_reach_later_frames() {
        let (core, sink) = app_sending(halves_jpeg(), 0, true);
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("first preview", |events| !previews(events).is_empty());
        assert!(
            previews(&events)[0].pixel(4, 16)[0] < 55,
            "starts unmirrored"
        );

        core.set_processing(settings(100.0, 0.0, true)).unwrap();
        sink.wait_for("a mirrored preview", |events| {
            previews(events)
                .iter()
                .any(|frame| frame.pixel(4, 16)[0] > 200)
        });
        core.stop().unwrap();
    }

    /// A renderer without H264, whatever this machine has installed.
    fn no_h264_renderer() -> Arc<dyn PreviewRenderer> {
        Arc::new(
            FxRenderer::new(Arc::new(NoFaces))
                .with_h264_unavailable("install OpenH264 to show H264"),
        )
    }

    fn app_using(renderer: Arc<dyn PreviewRenderer>) -> (AppCore, Arc<Collector>) {
        let sink = Arc::new(Collector::default());
        let catalog = Arc::new(FakeCatalog {
            frames: 1,
            endless: false,
            repeat: false,
            payload: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mode: mode(30),
        });
        (
            AppCore::with_renderer(catalog, sink.clone(), AppConfig::default(), renderer),
            sink,
        )
    }

    #[test]
    fn probe_hides_formats_the_preview_cannot_show_and_says_why() {
        let (core, _sink) = app_using(no_h264_renderer());
        let probe = core.probe(2).unwrap();
        let formats: std::collections::BTreeSet<String> = probe
            .capabilities
            .modes()
            .iter()
            .map(|mode| mode.format.to_string())
            .collect();
        assert_eq!(formats, ["MJPG".to_string(), "YUYV".to_string()].into());
        // Two H264 frame rates at one size collapse into one explanation.
        assert_eq!(probe.skipped.len(), 1);
        assert_eq!(probe.skipped[0].format, "H264");
        assert_eq!(probe.skipped[0].size, Some((1280, 720)));
        assert!(probe.skipped[0].reason.contains("install OpenH264"));
    }

    #[test]
    fn starting_a_format_the_preview_cannot_show_is_a_typed_error() {
        let (core, sink) = app_using(no_h264_renderer());
        let error = core.start(2, mode_of("H264", 1280, 720, 30)).unwrap_err();
        assert_eq!(error.code(), "format_not_previewable");
        assert!(!core.is_running());
        assert!(sink.events().is_empty());
    }

    // ---- stateful streams (H264) ----

    #[derive(Clone, Default)]
    struct Counting {
        renders: Arc<std::sync::atomic::AtomicUsize>,
        advances: Arc<std::sync::atomic::AtomicUsize>,
        delay_ms: u64,
    }

    impl PreviewRenderer for Counting {
        fn start(&self) -> Box<dyn crate::preview::PreviewSession> {
            Box::new(self.clone())
        }
        fn supports(&self, _fourcc: &str) -> Result<(), String> {
            Ok(())
        }
    }

    impl crate::preview::PreviewSession for Counting {
        fn advance(
            &mut self,
            _timestamp_us: u64,
            _mode: &CaptureMode,
            _data: &[u8],
        ) -> Result<(), String> {
            std::thread::sleep(Duration::from_millis(self.delay_ms));
            self.advances.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn render(
            &mut self,
            _timestamp_us: u64,
            _settings: &ProcessingSettings,
            _mode: &CaptureMode,
            _data: &[u8],
        ) -> Result<crate::preview::Rendered, String> {
            std::thread::sleep(Duration::from_millis(self.delay_ms));
            self.renders.fetch_add(1, Ordering::SeqCst);
            Ok(crate::preview::Rendered {
                jpeg: vec![0xFF, 0xD8, 0xFF, 0xD9],
                auto_framing: crate::preview::AutoFramingStatus::Off,
                background: crate::preview::BackgroundStatus::Off,
            })
        }
    }

    fn run_counting(
        camera_mode: CaptureMode,
        frames: u64,
        renderer: &Counting,
        queue: usize,
    ) -> Vec<AppEvent> {
        run_counting_with(
            camera_mode,
            frames,
            renderer,
            queue,
            settings(100.0, 0.0, true),
        )
    }

    fn run_counting_with(
        camera_mode: CaptureMode,
        frames: u64,
        renderer: &Counting,
        queue: usize,
        processing: ProcessingSettings,
    ) -> Vec<AppEvent> {
        let sink = Arc::new(Collector::default());
        let config = AppConfig {
            preview_interval_us: 60_000,
            stats_interval_us: 100_000,
            preview_queue_depth: queue,
            session: SessionConfig {
                max_frames: Some(frames),
                ..SessionConfig::default()
            },
        };
        let catalog = Arc::new(FakeCatalog {
            frames,
            endless: false,
            repeat: false,
            payload: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mode: camera_mode,
        });
        let core =
            AppCore::with_renderer(catalog, sink.clone(), config, Arc::new(renderer.clone()));
        core.set_processing(processing).unwrap();
        core.start(2, camera_mode).unwrap();
        sink.wait_for("end", Collector::ended)
    }

    #[test]
    fn a_stateless_stream_is_only_rendered_when_a_preview_frame_is_due() {
        let renderer = Counting::default();
        run_counting(mode(30), 20, &renderer, 16);
        assert_eq!(renderer.renders.load(Ordering::SeqCst), 10);
        assert_eq!(renderer.advances.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn with_the_virtual_camera_on_every_frame_is_processed_not_only_the_preview_ones() {
        let renderer = Counting::default();
        let mut on_air = settings(100.0, 0.0, true);
        on_air.virtual_camera.enabled = true;
        run_counting_with(mode(30), 12, &renderer, 16, on_air);
        assert_eq!(renderer.renders.load(Ordering::SeqCst), 12);
    }

    #[test]
    fn a_stateful_stream_reaches_the_decoder_frame_by_frame_and_is_shown_only_when_due() {
        let renderer = Counting::default();
        // Fewer frames than the queue holds, as the source here does not wait.
        let events = run_counting(mode_of("H264", 128, 64, 30), 12, &renderer, 16);
        let renders = renderer.renders.load(Ordering::SeqCst);
        let advances = renderer.advances.load(Ordering::SeqCst);
        assert_eq!(renders, 6, "the frames due for the preview");
        assert_eq!(renders + advances, 12, "every frame reached the decoder");
        let dropped = events
            .iter()
            .filter(|event| matches!(event, AppEvent::PreviewDropped { .. }))
            .count();
        assert_eq!(dropped, 0);
    }

    #[test]
    fn frames_a_busy_decoder_could_not_take_are_counted_as_dropped() {
        let renderer = Counting {
            delay_ms: 20,
            ..Counting::default()
        };
        let events = run_counting(mode_of("H264", 128, 64, 30), 60, &renderer, 1);
        let highest = events
            .iter()
            .filter_map(|event| match event {
                AppEvent::PreviewDropped { total } => Some(*total),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        assert!(
            highest >= 10,
            "dropped {highest}: the queue holds 8 and the source does not wait"
        );
        match events.last() {
            Some(AppEvent::Ended {
                outcome: Some(outcome),
                ..
            }) => {
                assert_eq!(
                    outcome.stats.frames_captured, 60,
                    "the capture never waited for the decoder"
                );
            }
            other => panic!("unexpected last event {other:?}"),
        }
    }

    #[test]
    fn probe_offers_a_format_the_renderer_can_show() {
        let (core, _sink) = app_using(Arc::new(Counting::default()));
        let formats: std::collections::BTreeSet<String> = core
            .probe(2)
            .unwrap()
            .capabilities
            .modes()
            .iter()
            .map(|mode| mode.format.to_string())
            .collect();
        assert!(formats.contains("H264"));
    }

    #[test]
    fn a_real_h264_camera_stream_is_shown_through_the_whole_core() {
        if lumencast_h264::availability().is_err() {
            eprintln!("SKIPPED: OpenH264 is not installed");
            return;
        }
        let keyframe = lumencast_h264::test_support::stream(1).unwrap().remove(0);
        let h264 = mode_of("H264", 128, 64, 30);
        let (core, sink) = app_sending_in(h264, keyframe, 6, false);
        core.start(2, h264).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        let shown = previews(&events);
        assert!(
            !shown.is_empty(),
            "no H264 picture reached the UI: {events:?}"
        );
        for frame in shown {
            assert!(frame.pixel(8, 32)[0] < 80 && frame.pixel(120, 32)[0] > 190);
        }
    }

    #[test]
    fn yuyv_cameras_are_shown_by_converting_their_frames() {
        // 64x32, left half black and right half white, as raw YUYV.
        let payload: Vec<u8> = (0..64 * 32 / 2)
            .flat_map(|pair| {
                let y = if pair % 32 < 16 { 16 } else { 235 };
                [y, 128, y, 128]
            })
            .collect();
        let yuyv = mode_of("YUYV", 64, 32, 30);
        let (core, sink) = app_sending_in(yuyv, payload, 4, false);
        core.start(2, yuyv).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        let shown = previews(&events);
        assert!(!shown.is_empty());
        for frame in shown {
            assert!(frame.pixel(4, 16)[0] < 80, "left should be dark");
            assert!(frame.pixel(59, 16)[0] > 200, "right should be bright");
        }
    }

    /// A renderer that takes 40 ms per frame, like a very slow machine.
    struct SlowRenderer;

    struct NoFaces;

    impl crate::preview::DetectorFactory for NoFaces {
        fn create(&self) -> Result<Box<dyn lumencast_fx::reframe::FaceDetector>, String> {
            Err("no detector in this test".into())
        }
    }

    struct SlowSession(Box<dyn crate::preview::PreviewSession>);

    impl crate::preview::PreviewSession for SlowSession {
        fn render(
            &mut self,
            timestamp_us: u64,
            settings: &ProcessingSettings,
            mode: &CaptureMode,
            data: &[u8],
        ) -> Result<crate::preview::Rendered, String> {
            std::thread::sleep(Duration::from_millis(40));
            self.0.render(timestamp_us, settings, mode, data)
        }
    }

    impl PreviewRenderer for SlowRenderer {
        fn start(&self) -> Box<dyn crate::preview::PreviewSession> {
            Box::new(SlowSession(FxRenderer::new(Arc::new(NoFaces)).start()))
        }
        fn supports(&self, fourcc: &str) -> Result<(), String> {
            FxRenderer::new(Arc::new(NoFaces)).supports(fourcc)
        }
    }

    #[test]
    fn a_slow_preview_skips_frames_but_never_slows_the_capture() {
        let sink = Arc::new(Collector::default());
        let config = AppConfig {
            preview_interval_us: 60_000,
            stats_interval_us: 100_000,
            preview_queue_depth: 1,
            session: SessionConfig {
                max_frames: Some(60),
                ..SessionConfig::default()
            },
        };
        let catalog = Arc::new(FakeCatalog {
            frames: 60,
            endless: false,
            repeat: false,
            payload: halves_jpeg(),
            mode: mode(30),
        });
        let core = AppCore::with_renderer(catalog, sink.clone(), config, Arc::new(SlowRenderer));
        core.set_processing(settings(100.0, 0.0, true)).unwrap();

        let started = Instant::now();
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("end", Collector::ended);
        // 30 previews are due, 40 ms each would be 1.2 s if the capture waited.
        let capture_time = started.elapsed();
        match events.last() {
            Some(AppEvent::Ended {
                outcome: Some(outcome),
                ..
            }) => {
                assert_eq!(outcome.stats.frames_captured, 60);
                assert!(
                    outcome.stats.frames_previewed < 15,
                    "the worker should have skipped frames, previewed {}",
                    outcome.stats.frames_previewed
                );
                assert!(outcome.stats.frames_previewed >= 1);
            }
            other => panic!("unexpected last event {other:?}"),
        }
        assert!(
            capture_time < Duration::from_millis(600),
            "capture took {capture_time:?}"
        );
        // Every previewed frame was announced before the capture ended.
        let announced = previews(&events).len() as u64;
        match events.last() {
            Some(AppEvent::Ended {
                outcome: Some(outcome),
                ..
            }) => assert_eq!(announced, outcome.stats.frames_previewed),
            other => panic!("unexpected last event {other:?}"),
        }
    }

    // ---- persistence and legacy import ----

    #[derive(Default)]
    struct MemoryStore {
        text: Mutex<Option<String>>,
        fail_save: std::sync::atomic::AtomicBool,
    }

    impl MemoryStore {
        fn holding(text: &str) -> Arc<Self> {
            Arc::new(Self {
                text: Mutex::new(Some(text.to_owned())),
                ..Self::default()
            })
        }
        fn json(&self) -> serde_json::Value {
            serde_json::from_str(self.text.lock().unwrap().as_deref().unwrap()).unwrap()
        }
    }

    impl SettingsStore for MemoryStore {
        fn load(&self) -> Result<Option<String>, String> {
            Ok(self.text.lock().unwrap().clone())
        }
        fn save(&self, text: &str) -> Result<(), String> {
            if self.fail_save.load(Ordering::Relaxed) {
                return Err("disk full".into());
            }
            *self.text.lock().unwrap() = Some(text.to_owned());
            Ok(())
        }
    }

    struct StaticLegacy(&'static str);

    impl LegacySource for StaticLegacy {
        fn read(&self) -> Option<String> {
            Some(self.0.to_owned())
        }
    }

    const LEGACY: &str = r#"{"zoom_factor": 157, "pan_x": 0.4, "pan_y": 1.0, "mirror_preview": true,
        "language": "es", "input_device": "/dev/video2", "resolution": "1280x720", "fps": 30,
        "effect_mode": "remove", "eye_contact": false}"#;

    fn app_with_storage(
        store: &Arc<MemoryStore>,
        legacy: Option<&'static str>,
    ) -> (AppCore, Arc<Collector>) {
        let sink = Arc::new(Collector::default());
        let catalog = Arc::new(FakeCatalog {
            frames: 1,
            endless: false,
            repeat: false,
            payload: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mode: mode(30),
        });
        let storage = Storage {
            store: store.clone(),
            legacy: legacy.map(|text| Arc::new(StaticLegacy(text)) as Arc<dyn LegacySource>),
            home: Some("/home/user".into()),
        };
        (
            AppCore::with_storage(catalog, sink.clone(), AppConfig::default(), storage),
            sink,
        )
    }

    /// Like `app_with_storage` but with fake audio devices, so nothing real is opened.
    fn app_with_audio(
        store: &Arc<MemoryStore>,
        io: &Arc<crate::audio::testing::FakeIo>,
    ) -> AppCore {
        let catalog = Arc::new(FakeCatalog {
            frames: 1,
            endless: false,
            repeat: false,
            payload: vec![0xFF, 0xD8, 0xFF, 0xD9],
            mode: mode(30),
        });
        let storage = Storage {
            store: store.clone(),
            legacy: None,
            home: Some("/home/user".into()),
        };
        AppCore::with_audio_io(
            catalog,
            Arc::new(Collector::default()),
            AppConfig::default(),
            storage,
            io.clone(),
        )
    }

    #[test]
    fn the_audio_settings_are_applied_saved_and_started_again_with_the_next_run() {
        let store = Arc::new(MemoryStore::default());
        let io = Arc::new(crate::audio::testing::FakeIo::default());
        let core = app_with_audio(&store, &io);
        assert_eq!(core.audio_status().0, crate::audio::PathStatus::Off);
        let mut settings = core.audio_settings();
        settings.microphone.enabled = true;
        settings.microphone.studio_voice = true;
        core.set_audio(settings.clone()).unwrap();
        assert_eq!(core.audio_status().0, crate::audio::PathStatus::Running);
        assert_eq!(io.opened.lock().unwrap().len(), 2);
        drop(core);

        // A new run reads the saved settings and starts the microphone by itself.
        let second_io = Arc::new(crate::audio::testing::FakeIo::default());
        let again = app_with_audio(&store, &second_io);
        assert_eq!(again.audio_settings(), settings);
        assert_eq!(again.audio_status().0, crate::audio::PathStatus::Running);
        assert_eq!(second_io.opened.lock().unwrap().len(), 2);
    }

    #[test]
    fn closing_to_the_tray_and_checking_for_updates_default_on_and_are_saved() {
        let store = Arc::new(MemoryStore::default());
        let io = Arc::new(crate::audio::testing::FakeIo::default());
        let core = app_with_audio(&store, &io);
        assert!(core.close_to_tray() && core.check_updates());
        core.set_close_to_tray(false).unwrap();
        core.set_check_updates(false).unwrap();
        drop(core);
        let again = app_with_audio(&store, &io);
        assert!(!again.close_to_tray() && !again.check_updates());
    }

    #[test]
    fn shutting_down_ends_the_audio_paths() {
        let store = Arc::new(MemoryStore::default());
        let io = Arc::new(crate::audio::testing::FakeIo::default());
        let core = app_with_audio(&store, &io);
        let mut settings = core.audio_settings();
        settings.microphone.enabled = true;
        core.set_audio(settings).unwrap();
        assert_eq!(core.audio_status().0, crate::audio::PathStatus::Running);
        core.shutdown();
        assert_eq!(core.audio_status().0, crate::audio::PathStatus::Off);
        // What was saved is untouched: the next run starts the microphone again.
        assert!(core.audio_settings().microphone.enabled);
    }

    #[test]
    fn invalid_audio_settings_are_refused_and_change_nothing() {
        let store = Arc::new(MemoryStore::default());
        let io = Arc::new(crate::audio::testing::FakeIo::default());
        let core = app_with_audio(&store, &io);
        let mut settings = core.audio_settings();
        settings.microphone.enabled = true;
        settings.microphone.noise_removal.strength = 200;
        assert_eq!(
            core.set_audio(settings).unwrap_err().code(),
            "invalid_setting"
        );
        assert_eq!(core.audio_status().0, crate::audio::PathStatus::Off);
        assert!(io.opened.lock().unwrap().is_empty());
    }

    #[test]
    fn the_first_run_imports_the_legacy_settings_once_and_saves_them() {
        let store = Arc::new(MemoryStore::default());
        let (core, _sink) = app_with_storage(&store, Some(LEGACY));
        let first = core.preferences();
        assert_eq!(first.settings.processing.zoom_percent, 157.0);
        assert!(first.settings.processing.mirror_preview);
        assert_eq!(first.settings.language.as_deref(), Some("es"));
        assert!(first.problem.is_none());
        let report = first.import_report.unwrap();
        assert!(report.applied.iter().any(|key| key == "zoom_factor"));
        assert!(report.kept_for_later.is_empty());
        assert!(report.applied.iter().any(|key| key == "effect_mode"));
        assert_eq!(report.ignored.len(), 1);
        assert!(
            core.preferences().import_report.is_none(),
            "reported only once"
        );
        assert_eq!(core.processing().zoom_percent, 157.0);
        assert_eq!(store.json()["processing"]["zoomPercent"], 157.0);

        // A later launch reads the saved file and does not import again.
        let (again, _sink) = app_with_storage(&store, Some(LEGACY));
        let preferences = again.preferences();
        assert!(preferences.import_report.is_none());
        assert_eq!(preferences.settings.processing.zoom_percent, 157.0);
    }

    #[test]
    fn without_a_saved_file_or_a_legacy_file_the_defaults_apply_and_nothing_is_written() {
        let store = Arc::new(MemoryStore::default());
        let (core, _sink) = app_with_storage(&store, None);
        let preferences = core.preferences();
        assert_eq!(
            preferences.settings,
            crate::settings::PersistedSettings::default()
        );
        assert!(preferences.import_report.is_none() && preferences.problem.is_none());
        assert!(store.text.lock().unwrap().is_none());
    }

    #[test]
    fn processing_changes_and_the_language_are_saved() {
        let store = Arc::new(MemoryStore::default());
        let (core, _sink) = app_with_storage(&store, None);
        core.set_processing(settings(140.0, 0.5, true)).unwrap();
        core.set_language("en").unwrap();
        let saved = store.json();
        assert_eq!(saved["processing"]["zoomPercent"], 140.0);
        assert_eq!(saved["processing"]["mirrorPreview"], true);
        assert_eq!(saved["language"], "en");
        assert_eq!(
            core.set_language("fr").unwrap_err().code(),
            "invalid_setting"
        );
        assert_eq!(store.json()["language"], "en");
    }

    #[test]
    fn starting_a_capture_remembers_the_camera_and_the_exact_mode() {
        let store = Arc::new(MemoryStore::default());
        let (core, sink) = app_with_storage(&store, None);
        core.start(2, mode(30)).unwrap();
        sink.wait_for("end", Collector::ended);
        let saved = store.json();
        assert_eq!(saved["preferredCamera"]["usbId"], "04e8:205d");
        assert_eq!(saved["preferredMode"]["format"], "MJPG");
        assert_eq!(saved["preferredMode"]["size"]["width"], 1280);
    }

    #[test]
    fn unreadable_or_invalid_saved_settings_fall_back_to_defaults_with_a_visible_problem() {
        for broken in ["{not json", r#"{"processing":{"zoomPercent":999}}"#] {
            let store = MemoryStore::holding(broken);
            let (core, _sink) = app_with_storage(&store, Some(LEGACY));
            let preferences = core.preferences();
            assert!(preferences.problem.is_some(), "{broken}");
            assert_eq!(
                preferences.settings.processing,
                ProcessingSettings::default()
            );
            assert!(
                preferences.import_report.is_none(),
                "must not import over a broken file"
            );
            assert_eq!(core.processing(), ProcessingSettings::default());
        }
    }

    #[test]
    fn a_failing_store_still_applies_the_setting_and_says_it_was_not_saved() {
        let store = Arc::new(MemoryStore::default());
        let (core, _sink) = app_with_storage(&store, None);
        store.fail_save.store(true, Ordering::Relaxed);
        let error = core
            .set_processing(settings(150.0, 0.0, false))
            .unwrap_err();
        assert_eq!(error.code(), "settings_not_saved");
        assert_eq!(core.processing().zoom_percent, 150.0);
    }

    #[test]
    fn the_standard_storage_uses_lumencast_and_reads_the_legacy_blucast_file() {
        let home = tempfile::tempdir().unwrap();
        let legacy_dir = home.path().join(".config/blucast");
        std::fs::create_dir_all(&legacy_dir).unwrap();
        std::fs::write(legacy_dir.join("settings.json"), LEGACY).unwrap();

        let storage = Storage::standard(None, home.path().to_str());
        assert_eq!(storage.home.as_deref(), home.path().to_str());
        assert_eq!(
            storage.legacy.as_ref().unwrap().read().as_deref(),
            Some(LEGACY)
        );
        storage.store.save("{}").unwrap();
        assert!(home
            .path()
            .join(".config/lumencast/settings.json")
            .is_file());
        // The legacy file is never touched.
        assert_eq!(
            std::fs::read_to_string(legacy_dir.join("settings.json")).unwrap(),
            LEGACY
        );
    }

    #[test]
    fn without_a_home_nothing_is_stored_and_nothing_fails() {
        let storage = Storage::standard(None, None);
        assert!(storage.legacy.is_none());
        assert_eq!(storage.store.load().unwrap(), None);
        assert!(storage.store.save("{}").is_ok());
    }

    // ---- auto framing through the whole core ----

    struct OneFace;

    impl lumencast_fx::reframe::FaceDetector for OneFace {
        fn detect(
            &mut self,
            _frame: &lumencast_fx::frame::Frame,
        ) -> Option<lumencast_fx::reframe::FaceBox> {
            Some(lumencast_fx::reframe::FaceBox {
                center_x: 48.0,
                center_y: 16.0,
                height: 6.0,
            })
        }
    }

    struct OneFaceFactory;

    impl crate::preview::DetectorFactory for OneFaceFactory {
        fn create(&self) -> Result<Box<dyn lumencast_fx::reframe::FaceDetector>, String> {
            Ok(Box::new(OneFace))
        }
    }

    fn auto_app(factory: Arc<dyn crate::preview::DetectorFactory>) -> (AppCore, Arc<Collector>) {
        let sink = Arc::new(Collector::default());
        let config = AppConfig {
            preview_interval_us: 60_000,
            stats_interval_us: 100_000,
            preview_queue_depth: 16,
            session: SessionConfig::default(),
        };
        let catalog = Arc::new(FakeCatalog {
            frames: 0,
            endless: true,
            repeat: true,
            payload: halves_jpeg(),
            mode: mode(30),
        });
        let core = AppCore::with_renderer(
            catalog,
            sink.clone(),
            config,
            Arc::new(FxRenderer::new(factory)),
        );
        let mut processing = ProcessingSettings::default();
        processing.auto_framing.enabled = true;
        processing.auto_framing.speed = 100;
        core.set_processing(processing).unwrap();
        (core, sink)
    }

    fn auto_statuses(events: &[AppEvent]) -> Vec<serde_json::Value> {
        events
            .iter()
            .filter(|event| matches!(event, AppEvent::AutoFraming { .. }))
            .map(|event| serde_json::to_value(event).unwrap())
            .collect()
    }

    #[test]
    fn the_ui_is_told_when_the_auto_framing_locks_onto_a_face() {
        let (core, sink) = auto_app(Arc::new(OneFaceFactory));
        core.start(2, mode(30)).unwrap();
        let events = sink.wait_for("a locked status", |events| {
            auto_statuses(events)
                .iter()
                .any(|status| status["status"]["state"] == "locked")
        });
        core.stop().unwrap();
        let locked = auto_statuses(&events)
            .into_iter()
            .find(|status| status["status"]["state"] == "locked")
            .unwrap();
        assert_eq!(locked["kind"], "autoFraming");
        assert!(locked["status"]["zoomPercent"].as_f64().unwrap() > 100.0);
        // The preview is encoded on its own thread, so its first frame can land
        // a moment after the status that was waited for.
        let events = sink.wait_for("a preview frame", |events| !previews(events).is_empty());
        assert!(
            !previews(&events).is_empty(),
            "the preview keeps coming while following"
        );
    }

    #[test]
    fn a_detector_that_cannot_start_is_announced_once() {
        let (core, sink) = auto_app(Arc::new(NoFaces));
        core.start(2, mode(30)).unwrap();
        sink.wait_for("an unavailable status", |events| {
            auto_statuses(events)
                .iter()
                .any(|status| status["status"]["state"] == "unavailable")
        });
        std::thread::sleep(Duration::from_millis(120));
        core.stop().unwrap();
        let statuses = auto_statuses(&sink.events());
        assert_eq!(statuses.len(), 1, "{statuses:?}");
    }
}
