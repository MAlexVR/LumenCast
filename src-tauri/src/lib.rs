//! Thin Tauri glue. Every command delegates to `AppCore`, which is tested
//! without Tauri; blocking work runs off the UI thread.

use lumencast_app::audio::devices::AudioDevice;
use lumencast_app::audio::{AudioSettings, PathStatus};
use lumencast_app::autostart::{Autostart, AUTOSTART_ARGUMENT};
use lumencast_app::core::{AppConfig, AppCore, Storage};
use lumencast_app::error::AppError;
use lumencast_app::events::{AppEvent, EventSink};
use lumencast_app::processing::ProcessingSettings;
use lumencast_app::settings::Preferences;
use lumencast_core::camera::{CameraInfo, CameraProbe};
use lumencast_core::error::ErrorPayload;
use lumencast_core::mode::CaptureMode;
use lumencast_v4l::device::V4lCatalog;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, State};

const CAPTURE_EVENT: &str = "capture://event";

mod tray;
mod update;

pub(crate) struct AppState {
    pub(crate) core: Arc<AppCore>,
    /// The tray's check items, once the tray exists.
    pub(crate) tray: std::sync::Mutex<Option<tray::TrayItems>>,
    /// Whether a tray icon was created: only then does closing the window hide it.
    pub(crate) tray_ready: std::sync::atomic::AtomicBool,
}

struct TauriSink(AppHandle);

impl EventSink for TauriSink {
    fn emit(&self, event: AppEvent) {
        if let Err(error) = self.0.emit(CAPTURE_EVENT, event) {
            eprintln!("lumencast: could not deliver a capture event: {error}");
        }
    }
}

/// Runs `work` on the blocking pool and maps its error to `{code, message}`.
async fn run_blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, ErrorPayload> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| ErrorPayload {
            code: "internal_error",
            message: error.to_string(),
        })?
        .map_err(|error| error.payload())
}

#[tauri::command]
async fn list_cameras(state: State<'_, AppState>) -> Result<Vec<CameraInfo>, ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || Ok(core.list_cameras())).await
}

#[tauri::command]
async fn probe_camera(
    state: State<'_, AppState>,
    index: usize,
) -> Result<CameraProbe, ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.probe(index)).await
}

#[tauri::command]
async fn start_capture(
    state: State<'_, AppState>,
    index: usize,
    mode: CaptureMode,
) -> Result<(), ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.start(index, mode)).await
}

#[tauri::command]
async fn get_processing(state: State<'_, AppState>) -> Result<ProcessingSettings, ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || Ok(core.processing())).await
}

#[tauri::command]
async fn set_processing(
    state: State<'_, AppState>,
    settings: ProcessingSettings,
) -> Result<(), ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.set_processing(settings)).await?;
    refresh_tray(&state);
    Ok(())
}

/// How the two audio paths are doing right now, for a window opened after
/// the status events were sent.
#[derive(serde::Serialize)]
struct AudioPaths {
    microphone: PathStatus,
    speakers: PathStatus,
}

#[tauri::command]
async fn get_audio_status(state: State<'_, AppState>) -> Result<AudioPaths, ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || {
        let (microphone, speakers) = core.audio_status();
        Ok(AudioPaths {
            microphone,
            speakers,
        })
    })
    .await
}

#[tauri::command]
async fn get_audio(state: State<'_, AppState>) -> Result<AudioSettings, ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || Ok(core.audio_settings())).await
}

#[tauri::command]
async fn set_audio(
    state: State<'_, AppState>,
    settings: AudioSettings,
) -> Result<(), ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.set_audio(settings)).await
}

#[tauri::command]
async fn list_audio_devices(state: State<'_, AppState>) -> Result<Vec<AudioDevice>, ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.audio_devices()).await
}

#[tauri::command]
async fn save_profile(state: State<'_, AppState>, path: String) -> Result<(), ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.save_profile(&path)).await
}

#[tauri::command]
async fn load_profile(
    state: State<'_, AppState>,
    path: String,
) -> Result<lumencast_app::profile::Profile, ErrorPayload> {
    let core = Arc::clone(&state.core);
    let profile = run_blocking(move || core.load_profile(&path)).await?;
    refresh_tray(&state);
    Ok(profile)
}

#[tauri::command]
async fn get_preferences(state: State<'_, AppState>) -> Result<Preferences, ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || Ok(core.preferences())).await
}

#[tauri::command]
async fn set_language(state: State<'_, AppState>, code: String) -> Result<(), ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.set_language(&code)).await
}

#[tauri::command]
async fn get_autostart() -> Result<bool, ErrorPayload> {
    run_blocking(move || Ok(standard_autostart().is_some_and(|autostart| autostart.is_enabled())))
        .await
}

#[tauri::command]
async fn set_autostart(state: State<'_, AppState>, enabled: bool) -> Result<(), ErrorPayload> {
    run_blocking(move || set_autostart_enabled(enabled)).await?;
    refresh_tray(&state);
    Ok(())
}

pub(crate) fn autostart_enabled() -> bool {
    standard_autostart().is_some_and(|autostart| autostart.is_enabled())
}

pub(crate) fn set_autostart_enabled(enabled: bool) -> Result<(), AppError> {
    let autostart = standard_autostart()
        .ok_or_else(|| AppError::AutostartFailed("there is no home or config directory".into()))?;
    if enabled {
        let executable = std::env::current_exe()
            .map_err(|error| AppError::AutostartFailed(error.to_string()))?;
        autostart.enable(&executable)
    } else {
        autostart.disable()
    }
}

/// Makes the tray's check marks follow what the window changed.
fn refresh_tray(state: &AppState) {
    if let Ok(items) = state.tray.lock() {
        if let Some(items) = items.as_ref() {
            tray::refresh(items, &state.core, autostart_enabled());
        }
    }
}

#[tauri::command]
async fn set_close_to_tray(state: State<'_, AppState>, enabled: bool) -> Result<(), ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.set_close_to_tray(enabled)).await
}

#[tauri::command]
async fn set_check_updates(state: State<'_, AppState>, enabled: bool) -> Result<(), ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.set_check_updates(enabled)).await
}

fn standard_autostart() -> Option<Autostart> {
    Autostart::standard(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

#[tauri::command]
async fn stop_capture(state: State<'_, AppState>) -> Result<(), ErrorPayload> {
    let core = Arc::clone(&state.core);
    run_blocking(move || core.stop()).await
}

pub fn run() {
    let result = tauri::Builder::default()
        // A second launch shows the window of the one that is running (it may be in the tray).
        .plugin(tauri_plugin_single_instance::init(
            |app, _arguments, _directory| {
                tray::show_window(app);
            },
        ))
        .plugin(tauri_plugin_dialog::init())
        .manage(update::UpdateCancel::default())
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle();
                let keep = app.try_state::<AppState>().is_some_and(|state| {
                    state.tray_ready.load(std::sync::atomic::Ordering::SeqCst)
                        && state.core.close_to_tray()
                });
                if keep {
                    api.prevent_close();
                    tray::hide_window(app);
                }
            }
        })
        .setup(|app| {
            let sink = Arc::new(TauriSink(app.handle().clone()));
            let storage = Storage::standard(
                std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
                std::env::var("HOME").ok().as_deref(),
            );
            let core = AppCore::with_storage(
                Arc::new(V4lCatalog::default()),
                sink,
                AppConfig::default(),
                storage,
            );
            let core = Arc::new(core);
            app.manage(AppState {
                core: Arc::clone(&core),
                tray: std::sync::Mutex::new(None),
                tray_ready: std::sync::atomic::AtomicBool::new(false),
            });
            match tray::install(app.handle(), &core, autostart_enabled()) {
                Ok(items) => {
                    if let Some(state) = app.try_state::<AppState>() {
                        if let Ok(mut slot) = state.tray.lock() {
                            *slot = Some(items);
                        }
                        state
                            .tray_ready
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                }
                Err(error) => eprintln!("lumencast: the tray icon could not be created: {error}"),
            }
            // Started by the session: come up out of the way, in the tray.
            if std::env::args().any(|argument| argument == AUTOSTART_ARGUMENT) {
                let in_tray = app
                    .try_state::<AppState>()
                    .is_some_and(|s| s.tray_ready.load(std::sync::atomic::Ordering::SeqCst));
                if in_tray && core.close_to_tray() {
                    tray::hide_window(app.handle());
                } else if let Some(window) = app.get_webview_window("main") {
                    let _ = window.minimize();
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_cameras,
            probe_camera,
            start_capture,
            stop_capture,
            get_processing,
            set_processing,
            get_audio,
            get_audio_status,
            set_audio,
            list_audio_devices,
            get_preferences,
            set_language,
            get_autostart,
            set_autostart,
            set_close_to_tray,
            set_check_updates,
            save_profile,
            load_profile,
            update::check_update,
            update::install_update,
            update::cancel_update
        ])
        .build(tauri::generate_context!());
    let app = match result {
        Ok(app) => app,
        Err(error) => {
            eprintln!("lumencast: the application failed to run: {error}");
            std::process::exit(1);
        }
    };
    app.run(|handle, event| {
        if matches!(event, tauri::RunEvent::Exit) {
            // The virtual audio devices are child processes: end them with the program.
            if let Some(state) = handle.try_state::<AppState>() {
                state.core.shutdown();
            }
            // The matte server holds GPU memory: let it go with the program.
            lumencast_nvidia::sidecar::stop(&lumencast_nvidia::sidecar::SystemPodman);
        }
    });
}
