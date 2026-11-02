//! The system tray icon: the program keeps running when its window is closed
//! (the camera, the virtual camera and the audio devices carry on), and the
//! icon's menu is the way back to the window and to the few things worth
//! changing without opening it. On Linux the tray is a status notifier: a
//! click opens the menu, it does not report clicks, so "Show" is always there.

use crate::AppState;
use lumencast_app::core::AppCore;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Emitter, Manager, Wry};

/// Sent to the window when the menu changed a setting it shows.
pub const CHANGED_EVENT: &str = "tray://changed";
/// Sent to the window when the menu asks to look for updates.
pub const UPDATES_EVENT: &str = "tray://updates";

const SHOW: &str = "show";
const CAMERA: &str = "camera";
const AUTOSTART: &str = "autostart";
const UPDATES: &str = "updates";
const QUIT: &str = "quit";

/// The menu texts in one language.
#[derive(Debug, PartialEq, Eq)]
pub struct Labels {
    pub show: &'static str,
    pub camera: &'static str,
    pub autostart: &'static str,
    pub updates: &'static str,
    pub quit: &'static str,
    pub tooltip: &'static str,
}

/// The texts for `language` (`es` or `en`); anything else is English.
pub fn labels(language: &str) -> Labels {
    if language == "es" {
        Labels {
            show: "Mostrar u ocultar LumenCast",
            camera: "Cámara virtual",
            autostart: "Iniciar con la sesión",
            updates: "Buscar actualizaciones…",
            quit: "Salir",
            tooltip: "LumenCast",
        }
    } else {
        Labels {
            show: "Show or hide LumenCast",
            camera: "Virtual camera",
            autostart: "Start at login",
            updates: "Check for updates…",
            quit: "Quit",
            tooltip: "LumenCast",
        }
    }
}

/// The language of the menu: the saved one, else the one of the session.
pub fn language_of(saved: Option<&str>, locale: Option<&str>) -> &'static str {
    match saved {
        Some("es") => "es",
        Some("en") => "en",
        _ if locale.is_some_and(|l| l.starts_with("es")) => "es",
        _ => "en",
    }
}

/// The two check items, kept to follow what the window changes.
pub struct TrayItems {
    camera: CheckMenuItem<Wry>,
    autostart: CheckMenuItem<Wry>,
}

/// Shows the window and brings it to the front.
pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    set_visible(app, true);
}

/// Hides the window; the program keeps running.
pub fn hide_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    set_visible(app, false);
}

fn set_visible(app: &AppHandle, visible: bool) {
    if let Some(state) = app.try_state::<AppState>() {
        state.core.set_preview_visible(visible);
    }
}

fn toggle_window(app: &AppHandle) {
    let showing = app
        .get_webview_window("main")
        .is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false));
    if showing {
        hide_window(app);
    } else {
        show_window(app);
    }
}

/// Updates the check marks from what is saved.
pub fn refresh(items: &TrayItems, core: &AppCore, autostart: bool) {
    let _ = items
        .camera
        .set_checked(core.processing().virtual_camera.enabled);
    let _ = items.autostart.set_checked(autostart);
}

/// Puts the menu on the icon again a little after it was created, twice.
///
/// The shell's tray extension reads the menu over DBus as soon as the icon
/// registers, and the menu's texts can arrive just after that first reading;
/// an extension that missed them shows rows with no text. Setting the menu again
/// makes the extension read it complete.
fn refresh_menu_later(app: &AppHandle, tray: &TrayIcon<Wry>, menu: &Menu<Wry>) {
    let (app, tray, menu) = (app.clone(), tray.clone(), menu.clone());
    std::thread::spawn(move || {
        for pause in [1200u64, 3000] {
            std::thread::sleep(std::time::Duration::from_millis(pause));
            let (tray, menu) = (tray.clone(), menu.clone());
            // The menu is a GTK object: it is only touched from the main thread.
            let _ = app.run_on_main_thread(move || {
                let _ = tray.set_menu(Some(menu));
            });
        }
    });
}

/// Creates the tray icon and its menu.
pub fn install(app: &AppHandle, core: &AppCore, autostart: bool) -> tauri::Result<TrayItems> {
    let language = language_of(
        core.preferences().settings.language.as_deref(),
        std::env::var("LANG").ok().as_deref(),
    );
    let text = labels(language);
    let show = MenuItem::with_id(app, SHOW, text.show, true, None::<&str>)?;
    let camera = CheckMenuItem::with_id(
        app,
        CAMERA,
        text.camera,
        true,
        core.processing().virtual_camera.enabled,
        None::<&str>,
    )?;
    let start = CheckMenuItem::with_id(
        app,
        AUTOSTART,
        text.autostart,
        true,
        autostart,
        None::<&str>,
    )?;
    let updates = MenuItem::with_id(app, UPDATES, text.updates, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, text.quit, true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &camera,
            &start,
            &updates,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip(text.tooltip)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = builder.build(app)?;
    refresh_menu_later(app, &tray, &menu);
    Ok(TrayItems {
        camera,
        autostart: start,
    })
}

fn on_menu(app: &AppHandle, id: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    match id {
        SHOW => toggle_window(app),
        CAMERA => {
            let mut settings = state.core.processing();
            settings.virtual_camera.enabled = !settings.virtual_camera.enabled;
            if let Err(error) = state.core.set_processing(settings) {
                eprintln!("lumencast: the tray could not change the virtual camera: {error}");
            }
            let _ = app.emit(CHANGED_EVENT, ());
        }
        AUTOSTART => {
            let enabled = !crate::autostart_enabled();
            if let Err(error) = crate::set_autostart_enabled(enabled) {
                eprintln!("lumencast: the tray could not change the autostart: {error}");
            }
            let _ = app.emit(CHANGED_EVENT, ());
        }
        UPDATES => {
            show_window(app);
            let _ = app.emit(UPDATES_EVENT, ());
        }
        QUIT => app.exit(0),
        _ => {}
    }
    let autostart = crate::autostart_enabled();
    let slot = state.tray.lock();
    if let Ok(items) = slot {
        if let Some(items) = items.as_ref() {
            refresh(items, &state.core, autostart);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_speaks_the_language_of_the_settings_then_of_the_session() {
        assert_eq!(labels("es").quit, "Salir");
        assert_eq!(labels("en").quit, "Quit");
        assert_eq!(labels("fr"), labels("en"));
        assert_eq!(language_of(Some("es"), Some("en_US.UTF-8")), "es");
        assert_eq!(language_of(Some("en"), Some("es_CO.UTF-8")), "en");
        assert_eq!(language_of(None, Some("es_CO.UTF-8")), "es");
        assert_eq!(language_of(None, Some("C")), "en");
        assert_eq!(language_of(None, None), "en");
    }
}
