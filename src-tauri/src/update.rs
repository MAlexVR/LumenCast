//! The commands of the program's own update: look for a newer version and
//! install it. The window never sends an address or a file name: the backend
//! reads the release again itself when installing.

use lumencast_core::error::ErrorPayload;
use lumencast_update::http::UreqFetch;
use lumencast_update::install::HostSystem;
use lumencast_update::verify::UPDATE_PUBKEY;
use lumencast_update::{UpdateError, UpdateStatus, Updater, UPDATE_REPO};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

pub const PROGRESS_EVENT: &str = "update://progress";
/// A file with an access token, for a private repository: `<config>/lumencast/update-token`.
const TOKEN_FILE: &str = "update-token";

/// Set by the window to stop a download in progress.
#[derive(Default)]
pub struct UpdateCancel(Arc<AtomicBool>);

fn token() -> Option<String> {
    let base = match (
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|v| !v.is_empty()),
        std::env::var("HOME").ok().filter(|v| !v.is_empty()),
    ) {
        (Some(xdg), _) => PathBuf::from(xdg),
        (None, Some(home)) => PathBuf::from(home).join(".config"),
        (None, None) => return None,
    };
    let text = std::fs::read_to_string(base.join("lumencast").join(TOKEN_FILE)).ok()?;
    let text = text.trim();
    (!text.is_empty() && text.len() <= 255).then(|| text.to_owned())
}

fn updater() -> Result<Updater, UpdateError> {
    Updater::new(
        UPDATE_REPO,
        env!("CARGO_PKG_VERSION"),
        UPDATE_PUBKEY,
        Box::new(UreqFetch::new(token())),
        Box::new(HostSystem),
    )
}

fn payload(error: &UpdateError) -> ErrorPayload {
    ErrorPayload {
        code: error.code(),
        message: error.to_string(),
    }
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, UpdateError> + Send + 'static,
) -> Result<T, ErrorPayload> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| ErrorPayload {
            code: "update_failed",
            message: error.to_string(),
        })?
        .map_err(|error| payload(&error))
}

#[tauri::command]
pub async fn check_update() -> Result<UpdateStatus, ErrorPayload> {
    blocking(|| updater()?.check(false)).await
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    cancel: State<'_, UpdateCancel>,
) -> Result<(), ErrorPayload> {
    let flag = Arc::clone(&cancel.0);
    flag.store(false, Ordering::SeqCst);
    let emitter = app.clone();
    blocking(move || {
        updater()?.install(
            false,
            &|progress| {
                let _ = emitter.emit(PROGRESS_EVENT, progress);
            },
            &flag,
        )
    })
    .await?;
    // The new version starts a moment from now; this one leaves first.
    app.exit(0);
    Ok(())
}

#[tauri::command]
pub fn cancel_update(cancel: State<'_, UpdateCancel>) {
    cancel.0.store(true, Ordering::SeqCst);
}
