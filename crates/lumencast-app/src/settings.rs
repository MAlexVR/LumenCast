//! Persisted settings and the one-time import of the legacy Blucast file.
//!
//! The legacy file is only ever read. Each key is validated; what cannot be
//! used is reported with the reason instead of being dropped silently.

use crate::audio::AudioSettings;
use crate::processing::{BackgroundMode, ProcessingSettings};
use lumencast_core::camera::CameraInfo;
use lumencast_core::mode::CaptureMode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const SETTINGS_VERSION: u32 = 1;

/// The camera the user last used, by identity that survives `/dev/video`
/// renumbering when the camera is a USB device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreferredCamera {
    pub usb_id: Option<String>,
    pub name: Option<String>,
}

/// A size and frame rate remembered from the legacy settings, which never
/// stored the pixel format. It is only a hint: the UI checks it against what
/// the camera really advertises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModeHint {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PersistedSettings {
    pub version: u32,
    pub processing: ProcessingSettings,
    /// `es` or `en`.
    pub language: Option<String>,
    pub preferred_camera: Option<PreferredCamera>,
    /// The exact mode of the last capture the user started.
    pub preferred_mode: Option<CaptureMode>,
    pub mode_hint: Option<ModeHint>,
    /// Legacy effect settings, kept untouched for when those effects exist.
    pub deferred: BTreeMap<String, serde_json::Value>,
    pub audio: AudioSettings,
    /// Closing the window keeps the program running in the system tray (the
    /// camera, the virtual camera and the audio devices keep working) instead
    /// of quitting.
    pub close_to_tray: bool,
    /// Look for a new version when the program starts.
    pub check_updates: bool,
}

impl Default for PersistedSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            processing: ProcessingSettings::default(),
            language: None,
            preferred_camera: None,
            preferred_mode: None,
            mode_hint: None,
            deferred: BTreeMap::new(),
            audio: AudioSettings::default(),
            close_to_tray: true,
            check_updates: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IgnoredSetting {
    pub key: String,
    pub reason: String,
}

/// What the import did, so the UI can tell the user once.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub applied: Vec<String>,
    pub kept_for_later: Vec<String>,
    pub ignored: Vec<IgnoredSetting>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LegacyImport {
    pub settings: PersistedSettings,
    pub report: ImportReport,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImportError {
    #[error("the legacy settings file is not a JSON object: {0}")]
    Invalid(String),
}

/// Where the app's own settings live.
pub trait SettingsStore: Send + Sync + 'static {
    /// `Ok(None)` means there is nothing saved yet.
    fn load(&self) -> Result<Option<String>, String>;
    fn save(&self, text: &str) -> Result<(), String>;
}

/// Where the legacy Blucast settings live. Read only on purpose.
pub trait LegacySource: Send + Sync + 'static {
    fn read(&self) -> Option<String>;
}

/// The user configuration directory: `$XDG_CONFIG_HOME` when it is an absolute
/// path (the spec ignores relative ones), otherwise `$HOME/.config`.
pub fn config_dir(xdg_config_home: Option<&str>, home: Option<&str>) -> Option<std::path::PathBuf> {
    let xdg = xdg_config_home
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute());
    xdg.or_else(|| {
        home.filter(|home| !home.is_empty())
            .map(|home| std::path::Path::new(home).join(".config"))
    })
}

/// Reads the legacy file; a missing or unreadable file counts as absent.
pub struct LegacyFile {
    path: std::path::PathBuf,
}

impl LegacyFile {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl LegacySource for LegacyFile {
    fn read(&self) -> Option<String> {
        std::fs::read_to_string(&self.path).ok()
    }
}

/// A store that keeps nothing; the default when no storage is configured.
pub struct NoStore;

impl SettingsStore for NoStore {
    fn load(&self) -> Result<Option<String>, String> {
        Ok(None)
    }

    fn save(&self, _text: &str) -> Result<(), String> {
        Ok(())
    }
}

/// A JSON file written atomically: the new content goes to a temporary file
/// next to it and is renamed into place, so a crash never leaves half a file.
pub struct FileStore {
    path: std::path::PathBuf,
}

impl FileStore {
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

impl SettingsStore for FileStore {
    fn load(&self) -> Result<Option<String>, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("{}: {error}", self.path.display())),
        }
    }

    fn save(&self, text: &str) -> Result<(), String> {
        let describe = |error: std::io::Error| format!("{}: {error}", self.path.display());
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(describe)?;
        }
        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        let temporary = std::path::PathBuf::from(temporary);
        std::fs::write(&temporary, text).map_err(describe)?;
        std::fs::rename(&temporary, &self.path).map_err(describe)
    }
}

/// What the UI needs at startup.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    pub settings: PersistedSettings,
    /// Present only the first time after an import, so the user is told once.
    pub import_report: Option<ImportReport>,
    /// Set when the saved settings could not be read or were invalid.
    pub problem: Option<String>,
}

/// `home` replaces the `/host_home/` prefix the container used for paths.
pub fn import_legacy(
    text: &str,
    home: Option<&str>,
    cameras: &[CameraInfo],
) -> Result<LegacyImport, ImportError> {
    let value: Value =
        serde_json::from_str(text).map_err(|error| ImportError::Invalid(error.to_string()))?;
    let Value::Object(entries) = value else {
        return Err(ImportError::Invalid(
            "the top level is not an object".into(),
        ));
    };
    let mut import = Import::default();
    for (key, value) in &entries {
        match key.as_str() {
            "language" => import.language(key, value),
            "mirror_preview" => import.mirror(key, value),
            "zoom_factor" => import.zoom(key, value),
            "pan_x" => import.pan(key, value, Axis::X),
            "pan_y" => import.pan(key, value, Axis::Y),
            "input_device" => import.camera(key, value, cameras),
            "resolution" => import.resolution(key, value),
            "fps" => import.fps(key, value),
            "auto_reframe" => import.auto_enabled(key, value),
            "autoreframe_zoom" => import.auto_zoom(key, value),
            "autoreframe_speed" => import.auto_speed(key, value),
            "autoreframe_model" => import.ignore(
                key,
                "LumenCast uses one built-in face detector, so the detector choice is not kept",
            ),
            "virtual_light" => import.light_enabled(key, value),
            "virtual_light_intensity" => import.light_intensity(key, value),
            "effect_mode" => import.background_mode(key, value),
            "blur_strength" => import.blur_strength(key, value),
            "background_image" => import.background_image(key, value, home),
            "eye_contact" | "eye_contact_intensity" => {
                import.ignore(key, "Eye Contact was removed from Blucast");
            }
            _ => import.ignore(key, "unknown key"),
        }
    }
    Ok(import.finish())
}

#[derive(Clone, Copy)]
enum Axis {
    X,
    Y,
}

#[derive(Default)]
struct Import {
    settings: PersistedSettings,
    report: ImportReport,
    size: Option<(u32, u32)>,
    fps: Option<u32>,
}

impl Import {
    fn ignore(&mut self, key: &str, reason: &str) {
        self.report.ignored.push(IgnoredSetting {
            key: key.to_owned(),
            reason: reason.to_owned(),
        });
    }

    fn apply(&mut self, key: &str) {
        self.report.applied.push(key.to_owned());
    }

    fn language(&mut self, key: &str, value: &Value) {
        match value.as_str() {
            Some(code @ ("es" | "en")) => {
                self.settings.language = Some(code.to_owned());
                self.apply(key);
            }
            _ => self.ignore(key, "expected \"es\" or \"en\""),
        }
    }

    fn mirror(&mut self, key: &str, value: &Value) {
        match value.as_bool() {
            Some(enabled) => {
                self.settings.processing.mirror_preview = enabled;
                self.apply(key);
            }
            None => self.ignore(key, "expected true or false"),
        }
    }

    fn zoom(&mut self, key: &str, value: &Value) {
        match value
            .as_f64()
            .filter(|percent| (100.0..=200.0).contains(percent))
        {
            Some(percent) => {
                self.settings.processing.zoom_percent = percent as f32;
                self.apply(key);
            }
            None => self.ignore(key, "expected a number from 100 to 200"),
        }
    }

    fn pan(&mut self, key: &str, value: &Value, axis: Axis) {
        match value.as_f64().filter(|pan| (-1.0..=1.0).contains(pan)) {
            Some(pan) => {
                match axis {
                    Axis::X => self.settings.processing.pan_x = pan as f32,
                    Axis::Y => self.settings.processing.pan_y = pan as f32,
                }
                self.apply(key);
            }
            None => self.ignore(key, "expected a number from -1 to 1"),
        }
    }

    fn auto_enabled(&mut self, key: &str, value: &Value) {
        match value.as_bool() {
            Some(enabled) => {
                self.settings.processing.auto_framing.enabled = enabled;
                self.apply(key);
            }
            None => self.ignore(key, "expected true or false"),
        }
    }

    fn background_mode(&mut self, key: &str, value: &Value) {
        let mode = match value.as_str() {
            Some("none") => BackgroundMode::None,
            Some("blur") => BackgroundMode::Blur,
            Some("remove") => BackgroundMode::Remove,
            Some("replace") => BackgroundMode::Replace,
            _ => {
                self.ignore(key, "expected none, blur, remove or replace");
                return;
            }
        };
        self.settings.processing.background.mode = mode;
        self.apply(key);
    }

    fn blur_strength(&mut self, key: &str, value: &Value) {
        match value
            .as_u64()
            .and_then(|level| u8::try_from(level).ok())
            .filter(|level| *level <= 100)
        {
            Some(level) => {
                self.settings.processing.background.blur_strength = level;
                self.apply(key);
            }
            None => self.ignore(key, "expected a whole number from 0 to 100"),
        }
    }

    fn background_image(&mut self, key: &str, value: &Value, home: Option<&str>) {
        let Some(path) = value.as_str().filter(|path| !path.is_empty()) else {
            self.ignore(key, "expected a file path");
            return;
        };
        // The container saw the user's home as /host_home.
        let path = match (home, path.strip_prefix("/host_home/")) {
            (Some(home), Some(rest)) => format!("{}/{rest}", home.trim_end_matches('/')),
            _ => path.to_owned(),
        };
        self.settings.processing.background.image_path = Some(path);
        self.apply(key);
    }

    fn light_enabled(&mut self, key: &str, value: &Value) {
        match value.as_bool() {
            Some(enabled) => {
                self.settings.processing.key_light.enabled = enabled;
                self.apply(key);
            }
            None => self.ignore(key, "expected true or false"),
        }
    }

    fn light_intensity(&mut self, key: &str, value: &Value) {
        match value
            .as_u64()
            .and_then(|level| u8::try_from(level).ok())
            .filter(|level| *level <= 100)
        {
            Some(level) => {
                self.settings.processing.key_light.intensity = level;
                self.apply(key);
            }
            None => self.ignore(key, "expected a whole number from 0 to 100"),
        }
    }

    fn auto_zoom(&mut self, key: &str, value: &Value) {
        match value
            .as_f64()
            .filter(|percent| (100.0..=200.0).contains(percent))
        {
            Some(percent) => {
                self.settings.processing.auto_framing.min_zoom_percent = percent as f32;
                self.apply(key);
            }
            None => self.ignore(key, "expected a number from 100 to 200"),
        }
    }

    fn auto_speed(&mut self, key: &str, value: &Value) {
        match value
            .as_u64()
            .and_then(|speed| u8::try_from(speed).ok())
            .filter(|speed| *speed <= 100)
        {
            Some(speed) => {
                self.settings.processing.auto_framing.speed = speed;
                self.apply(key);
            }
            None => self.ignore(key, "expected a whole number from 0 to 100"),
        }
    }

    fn camera(&mut self, key: &str, value: &Value, cameras: &[CameraInfo]) {
        let Some(path) = value.as_str() else {
            return self.ignore(key, "expected a device path");
        };
        match cameras.iter().find(|camera| camera.path == path) {
            Some(camera) => {
                self.settings.preferred_camera = Some(PreferredCamera {
                    usb_id: camera.usb_id.clone(),
                    name: camera.name.clone(),
                });
                self.apply(key);
            }
            None => self.ignore(
                key,
                &format!("{path} is not a camera now; device numbers change between sessions"),
            ),
        }
    }

    fn resolution(&mut self, key: &str, value: &Value) {
        let parsed = value.as_str().and_then(|text| {
            let (width, height) = text.split_once('x')?;
            Some((width.parse::<u32>().ok()?, height.parse::<u32>().ok()?))
        });
        match parsed.filter(|(width, height)| *width > 0 && *height > 0) {
            Some(size) => self.size = Some(size),
            None => self.ignore(key, "expected WIDTHxHEIGHT"),
        }
    }

    fn fps(&mut self, key: &str, value: &Value) {
        match value
            .as_u64()
            .and_then(|fps| u32::try_from(fps).ok())
            .filter(|fps| *fps > 0)
        {
            Some(fps) => self.fps = Some(fps),
            None => self.ignore(key, "expected a whole number above 0"),
        }
    }

    fn finish(mut self) -> LegacyImport {
        match (self.size, self.fps) {
            (Some((width, height)), Some(fps)) => {
                self.settings.mode_hint = Some(ModeHint { width, height, fps });
                self.apply("resolution");
                self.apply("fps");
            }
            (Some(_), None) => self.ignore("resolution", "a size is only useful together with fps"),
            (None, Some(_)) => {
                self.ignore("fps", "a frame rate is only useful together with a size")
            }
            (None, None) => {}
        }
        LegacyImport {
            settings: self.settings,
            report: self.report,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "effect_mode": "remove",
      "background_image": "/host_home/Pictures/wall.png",
      "blur_strength": 50,
      "resolution": "1280x720",
      "fps": 30,
      "input_device": "/dev/video2",
      "mirror_preview": true,
      "zoom_factor": 157,
      "pan_x": 0.4,
      "pan_y": 1.0,
      "auto_reframe": true,
      "autoreframe_model": "dnn",
      "autoreframe_zoom": 135,
      "autoreframe_speed": 80,
      "virtual_light": true,
      "virtual_light_intensity": 40,
      "language": "es",
      "eye_contact": false,
      "eye_contact_intensity": 16
    }"#;

    fn cameras() -> Vec<CameraInfo> {
        vec![
            CameraInfo {
                path: "/dev/video0".into(),
                index: 0,
                name: Some("Integrated Camera".into()),
                usb_id: Some("5986:115f".into()),
            },
            CameraInfo {
                path: "/dev/video2".into(),
                index: 2,
                name: Some("SAMSUNG VD USB HDCAM".into()),
                usb_id: Some("04e8:205d".into()),
            },
        ]
    }

    fn import(text: &str) -> LegacyImport {
        import_legacy(text, Some("/home/user"), &cameras()).unwrap()
    }

    fn ignored_keys(report: &ImportReport) -> Vec<&str> {
        report
            .ignored
            .iter()
            .map(|entry| entry.key.as_str())
            .collect()
    }

    #[test]
    fn a_real_shaped_file_keeps_what_is_useful() {
        let imported = import(SAMPLE);
        let settings = &imported.settings;
        assert_eq!(settings.language.as_deref(), Some("es"));
        assert_eq!(
            settings.processing,
            ProcessingSettings {
                zoom_percent: 157.0,
                pan_x: 0.4,
                pan_y: 1.0,
                mirror_preview: true,
                vignette: 0.0,
                vignette_follows_face: false,
                auto_framing: crate::processing::AutoFramingSettings {
                    enabled: true,
                    min_zoom_percent: 135.0,
                    speed: 80,
                },
                key_light: crate::processing::KeyLightSettings {
                    enabled: true,
                    intensity: 40,
                    warmth: 60,
                    light_picture: None,
                },
                background: crate::processing::BackgroundSettings {
                    engine: crate::processing::BackgroundEngine::Nvidia,
                    mode: BackgroundMode::Remove,
                    blur_strength: 50,
                    image_path: Some("/home/user/Pictures/wall.png".into()),
                },
                denoise: crate::processing::DenoiseSettings::default(),
                virtual_camera: crate::processing::VirtualCameraSettings::default(),
            }
        );
        assert_eq!(
            settings.preferred_camera,
            Some(PreferredCamera {
                usb_id: Some("04e8:205d".into()),
                name: Some("SAMSUNG VD USB HDCAM".into()),
            })
        );
        assert_eq!(
            settings.mode_hint,
            Some(ModeHint {
                width: 1280,
                height: 720,
                fps: 30
            })
        );
        for key in [
            "language",
            "mirror_preview",
            "zoom_factor",
            "pan_x",
            "pan_y",
            "input_device",
            "resolution",
            "fps",
            "auto_reframe",
            "autoreframe_zoom",
            "autoreframe_speed",
        ] {
            assert!(
                imported.report.applied.iter().any(|applied| applied == key),
                "{key}"
            );
        }
    }

    #[test]
    fn every_legacy_effect_setting_is_a_real_setting_now_and_the_image_path_is_translated() {
        let imported = import(SAMPLE);
        let background = &imported.settings.processing.background;
        assert_eq!(background.mode, BackgroundMode::Remove);
        assert_eq!(background.blur_strength, 50);
        assert_eq!(
            background.image_path.as_deref(),
            Some("/home/user/Pictures/wall.png")
        );
        assert!(imported.settings.deferred.is_empty());
        assert!(imported.report.kept_for_later.is_empty());
    }

    #[test]
    fn a_bad_effect_mode_or_blur_is_ignored_with_a_reason() {
        let imported =
            import(r#"{"effect_mode": "sepia", "blur_strength": 400, "background_image": ""}"#);
        assert_eq!(
            imported.settings.processing.background,
            crate::processing::BackgroundSettings::default()
        );
        assert_eq!(imported.report.ignored.len(), 3);
    }

    #[test]
    fn eye_contact_and_unknown_keys_are_ignored_with_a_reason() {
        let imported =
            import(r#"{"eye_contact": true, "eye_contact_intensity": 3, "brand_new": 1}"#);
        assert_eq!(
            ignored_keys(&imported.report),
            ["eye_contact", "eye_contact_intensity", "brand_new"]
        );
        assert!(imported.report.ignored[0].reason.contains("removed"));
        assert!(imported.report.ignored[2].reason.contains("unknown"));
        assert_eq!(imported.settings, PersistedSettings::default());
    }

    #[test]
    fn invalid_values_are_ignored_one_by_one_without_affecting_the_valid_ones() {
        let imported = import(
            r#"{"zoom_factor": 250, "pan_x": "left", "pan_y": 0.5, "mirror_preview": "yes",
                "language": "fr", "resolution": "big", "fps": 0}"#,
        );
        assert_eq!(
            ignored_keys(&imported.report),
            [
                "zoom_factor",
                "pan_x",
                "mirror_preview",
                "language",
                "resolution",
                "fps"
            ]
        );
        assert_eq!(imported.settings.processing.pan_y, 0.5);
        assert_eq!(imported.settings.processing.zoom_percent, 100.0);
        assert!(!imported.settings.processing.mirror_preview);
        assert_eq!(imported.settings.language, None);
        assert_eq!(imported.settings.mode_hint, None);
    }

    #[test]
    fn auto_framing_keys_are_validated_and_the_detector_choice_is_reported() {
        let imported = import(
            r#"{"auto_reframe": "yes", "autoreframe_zoom": 250, "autoreframe_speed": 120, "autoreframe_model": "haar"}"#,
        );
        assert_eq!(
            ignored_keys(&imported.report),
            [
                "auto_reframe",
                "autoreframe_zoom",
                "autoreframe_speed",
                "autoreframe_model"
            ]
        );
        assert!(imported.report.ignored[3].reason.contains("detector"));
        assert_eq!(
            imported.settings.processing.auto_framing,
            crate::processing::AutoFramingSettings::default()
        );
    }

    #[test]
    fn a_device_node_that_is_no_longer_a_camera_is_reported() {
        let imported = import(r#"{"input_device": "/dev/video7"}"#);
        assert_eq!(imported.settings.preferred_camera, None);
        assert_eq!(ignored_keys(&imported.report), ["input_device"]);
        assert!(imported.report.ignored[0].reason.contains("/dev/video7"));
    }

    #[test]
    fn a_file_that_is_not_a_json_object_is_an_error() {
        assert!(matches!(
            import_legacy("not json", None, &[]),
            Err(ImportError::Invalid(_))
        ));
        assert!(matches!(
            import_legacy("[1, 2]", None, &[]),
            Err(ImportError::Invalid(_))
        ));
    }

    #[test]
    fn the_config_directory_follows_the_xdg_rules() {
        use std::path::PathBuf;
        assert_eq!(
            config_dir(Some("/custom/cfg"), Some("/home/u")),
            Some(PathBuf::from("/custom/cfg"))
        );
        assert_eq!(
            config_dir(None, Some("/home/u")),
            Some(PathBuf::from("/home/u/.config"))
        );
        assert_eq!(
            config_dir(Some(""), Some("/home/u")),
            Some(PathBuf::from("/home/u/.config"))
        );
        assert_eq!(
            config_dir(Some("relative/dir"), Some("/home/u")),
            Some(PathBuf::from("/home/u/.config"))
        );
        assert_eq!(config_dir(None, Some("")), None);
        assert_eq!(config_dir(None, None), None);
    }

    #[test]
    fn the_legacy_file_reads_text_and_treats_a_missing_file_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(LegacyFile::new(dir.path().join("none.json")).read(), None);
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{\"a\":1}").unwrap();
        assert_eq!(LegacyFile::new(&path).read().as_deref(), Some("{\"a\":1}"));
    }

    #[test]
    fn the_file_store_creates_folders_and_replaces_the_file_without_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/lumencast/settings.json");
        let store = FileStore::new(&path);
        assert_eq!(store.load().unwrap(), None);
        store.save("{\"a\":1}").unwrap();
        store.save("{\"a\":2}").unwrap();
        assert_eq!(store.load().unwrap().as_deref(), Some("{\"a\":2}"));
        let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["settings.json"]);
    }

    #[test]
    fn the_file_store_reports_an_unreadable_path_instead_of_hiding_it() {
        let dir = tempfile::tempdir().unwrap();
        // The "file" is a directory, so reading it must fail loudly.
        let store = FileStore::new(dir.path());
        assert!(store.load().is_err());
    }

    #[test]
    fn persisted_settings_round_trip_and_tolerate_missing_and_extra_fields() {
        let imported = import(SAMPLE);
        let text = serde_json::to_string(&imported.settings).unwrap();
        assert_eq!(
            serde_json::from_str::<PersistedSettings>(&text).unwrap(),
            imported.settings
        );
        let sparse: PersistedSettings =
            serde_json::from_str(r#"{"language": "en", "somethingNew": 1}"#).unwrap();
        assert_eq!(sparse.language.as_deref(), Some("en"));
        assert_eq!(sparse.version, SETTINGS_VERSION);
        assert_eq!(sparse.processing, ProcessingSettings::default());
    }
}
