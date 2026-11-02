//! A profile: the configuration of the effects and the audio, saved to a file
//! and loaded back, so a setup can be kept, moved to another machine or shared.
//!
//! What belongs to the machine or to the program and not to the setup is left
//! out: the chosen devices (a microphone or an output that another machine does
//! not have), the language, the autostart, the tray and the update settings.

use crate::audio::AudioSettings;
use crate::error::AppError;
use crate::processing::ProcessingSettings;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "lumencast-profile";
pub const VERSION: u32 = 1;
/// The largest profile file read: a real one is a few kilobytes.
const MAX_BYTES: u64 = 256 * 1024;
const EXTENSION: &str = "json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub format: String,
    pub version: u32,
    pub processing: ProcessingSettings,
    pub audio: AudioSettings,
}

impl Profile {
    /// A profile of the current settings, without the devices.
    pub fn of(processing: &ProcessingSettings, audio: &AudioSettings) -> Self {
        let mut audio = audio.clone();
        audio.microphone.source = None;
        audio.speakers.sink = None;
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            processing: processing.clone(),
            audio,
        }
    }

    pub fn to_text(&self) -> Result<String, AppError> {
        serde_json::to_string_pretty(self).map_err(|error| AppError::ProfileFile(error.to_string()))
    }

    /// Reads and checks a profile: its kind, its version and every value.
    pub fn parse(text: &str) -> Result<Self, AppError> {
        let profile: Self = serde_json::from_str(text).map_err(|error| {
            AppError::InvalidSetting(format!("not a LumenCast profile: {error}"))
        })?;
        if profile.format != FORMAT {
            return Err(AppError::InvalidSetting(format!(
                "not a LumenCast profile (format {:?})",
                profile.format
            )));
        }
        if profile.version == 0 || profile.version > VERSION {
            return Err(AppError::InvalidSetting(format!(
                "the profile is of version {}, this program reads up to {VERSION}",
                profile.version
            )));
        }
        profile.processing.validate()?;
        profile.audio.validate()?;
        Ok(profile)
    }
}

/// `path` with the `.json` extension, absolute and with an existing folder.
pub fn checked_path(path: &str) -> Result<PathBuf, AppError> {
    let path = Path::new(path);
    if !path.is_absolute() {
        return Err(AppError::ProfileFile("the path is not absolute".into()));
    }
    let path = if path.extension().and_then(|e| e.to_str()) == Some(EXTENSION) {
        path.to_owned()
    } else {
        let mut name = path.as_os_str().to_owned();
        name.push(format!(".{EXTENSION}"));
        PathBuf::from(name)
    };
    if path.parent().is_none_or(|folder| !folder.is_dir()) {
        return Err(AppError::ProfileFile("the folder does not exist".into()));
    }
    Ok(path)
}

/// Writes `profile` to `path` without leaving a half-written file.
pub fn write(path: &Path, profile: &Profile) -> Result<(), AppError> {
    let text = profile.to_text()?;
    let mut staging = path.as_os_str().to_owned();
    staging.push(".part");
    let staging = PathBuf::from(staging);
    std::fs::write(&staging, text).map_err(|e| AppError::ProfileFile(e.kind().to_string()))?;
    std::fs::rename(&staging, path).map_err(|e| {
        let _ = std::fs::remove_file(&staging);
        AppError::ProfileFile(e.kind().to_string())
    })
}

pub fn read(path: &Path) -> Result<Profile, AppError> {
    let size = std::fs::metadata(path)
        .map_err(|e| AppError::ProfileFile(e.kind().to_string()))?
        .len();
    if size > MAX_BYTES {
        return Err(AppError::InvalidSetting(
            "the file is too large to be a profile".into(),
        ));
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| AppError::ProfileFile(e.kind().to_string()))?;
    Profile::parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tuned() -> (ProcessingSettings, AudioSettings) {
        let mut processing = ProcessingSettings {
            zoom_percent: 140.0,
            ..ProcessingSettings::default()
        };
        processing.key_light.enabled = true;
        processing.key_light.intensity = 70;
        let mut audio = AudioSettings::default();
        audio.microphone.enabled = true;
        audio.microphone.source = Some("alsa_input.usb-x".into());
        audio.microphone.studio_voice = true;
        audio.speakers.sink = Some("alsa_output.pci-y".into());
        (processing, audio)
    }

    #[test]
    fn a_profile_keeps_the_setup_and_leaves_the_devices_out() {
        let (processing, audio) = tuned();
        let profile = Profile::of(&processing, &audio);
        assert_eq!(profile.processing, processing);
        assert!(profile.audio.microphone.studio_voice && profile.audio.microphone.enabled);
        assert_eq!(profile.audio.microphone.source, None);
        assert_eq!(profile.audio.speakers.sink, None);
        // What it was made from is untouched.
        assert!(audio.microphone.source.is_some());
    }

    #[test]
    fn what_is_written_is_read_back_the_same() {
        let (processing, audio) = tuned();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mine.json");
        let profile = Profile::of(&processing, &audio);
        write(&path, &profile).unwrap();
        assert_eq!(read(&path).unwrap(), profile);
        assert!(
            !dir.path().join("mine.json.part").exists(),
            "no staging file is left"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"format\": \"lumencast-profile\""));
    }

    #[test]
    fn a_file_that_is_not_a_profile_or_is_not_valid_is_refused_with_a_reason() {
        let valid = Profile::of(&ProcessingSettings::default(), &AudioSettings::default());
        let text = valid.to_text().unwrap();
        for bad in [
            "not json".to_owned(),
            "{}".to_owned(),
            text.replace("lumencast-profile", "something-else"),
            text.replace("\"version\": 1", "\"version\": 99"),
            text.replace("\"version\": 1", "\"version\": 0"),
            text.replace("\"zoomPercent\": 100.0", "\"zoomPercent\": 900.0"),
            text.replace("\"strength\": 80", "\"strength\": 300"),
        ] {
            assert!(
                matches!(Profile::parse(&bad), Err(AppError::InvalidSetting(_))),
                "{bad:.60}"
            );
        }
        assert!(Profile::parse(&text).is_ok());
    }

    #[test]
    fn fields_a_later_version_adds_are_ignored_and_missing_ones_take_their_default() {
        let text = r#"{"format":"lumencast-profile","version":1,"somethingNew":true,
                       "processing":{"zoomPercent":120.0},"audio":{}}"#;
        let profile = Profile::parse(text).unwrap();
        assert_eq!(profile.processing.zoom_percent, 120.0);
        assert_eq!(profile.audio, AudioSettings::default());
    }

    #[test]
    fn a_file_that_is_too_large_is_not_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.json");
        std::fs::write(&path, vec![b' '; (MAX_BYTES + 1) as usize]).unwrap();
        assert!(matches!(read(&path), Err(AppError::InvalidSetting(_))));
        assert!(matches!(
            read(&dir.path().join("missing.json")),
            Err(AppError::ProfileFile(_))
        ));
    }

    #[test]
    fn the_path_must_be_absolute_in_an_existing_folder_and_ends_in_json() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().to_string_lossy().into_owned();
        assert_eq!(
            checked_path(&format!("{base}/mine")).unwrap(),
            dir.path().join("mine.json")
        );
        assert_eq!(
            checked_path(&format!("{base}/mine.json")).unwrap(),
            dir.path().join("mine.json")
        );
        assert!(checked_path("relative/mine.json").is_err());
        assert!(checked_path("/no/such/folder/mine.json").is_err());
    }
}
