//! The audio settings: a virtual microphone and virtual speakers, each with
//! the effects NVIDIA Broadcast offers there (noise removal, room echo removal
//! and, for the microphone, studio voice).

use crate::error::AppError;
use lumencast_audio::chain::ChainSettings;
use lumencast_audio::voice::MicProfile;
use serde::{Deserialize, Serialize};

/// The name of the virtual microphone other programs see.
pub const MICROPHONE_NODE: &str = "lumencast_microphone";
pub const MICROPHONE_NAME: &str = "LumenCast Microphone";
/// The virtual output programs play into, to have it cleaned before it is heard.
pub const SPEAKERS_NODE: &str = "lumencast_speakers";
pub const SPEAKERS_NAME: &str = "LumenCast Speakers";

/// One effect with its strength (0 to 100).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Effect {
    pub enabled: bool,
    pub strength: u8,
}

impl Effect {
    pub const fn new(enabled: bool, strength: u8) -> Self {
        Self { enabled, strength }
    }

    /// The share to apply (0..=1), or `None` while it is off.
    pub fn level(&self) -> Option<f32> {
        (self.enabled && self.strength > 0).then(|| f32::from(self.strength) / 100.0)
    }

    fn validate(&self, what: &str) -> Result<(), AppError> {
        if self.strength > 100 {
            return Err(AppError::InvalidSetting(format!(
                "the {what} strength must be between 0 and 100, got {}",
                self.strength
            )));
        }
        Ok(())
    }
}

impl Default for Effect {
    fn default() -> Self {
        Self::new(false, 70)
    }
}

/// The kind of microphone, for the studio voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Profile {
    #[default]
    Headset,
    Laptop,
    Desk,
}

impl From<Profile> for MicProfile {
    fn from(profile: Profile) -> Self {
        match profile {
            Profile::Headset => MicProfile::Headset,
            Profile::Laptop => MicProfile::Laptop,
            Profile::Desk => MicProfile::Desk,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MicrophoneSettings {
    /// Publish the virtual microphone, fed by `source` through the effects.
    pub enabled: bool,
    /// The real microphone (a PipeWire node name); `None` is the system default.
    pub source: Option<String>,
    pub noise_removal: Effect,
    pub room_echo_removal: Effect,
    pub studio_voice: bool,
    pub profile: Profile,
}

impl Default for MicrophoneSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            source: None,
            noise_removal: Effect::new(true, 80),
            room_echo_removal: Effect::new(false, 50),
            studio_voice: false,
            profile: Profile::Headset,
        }
    }
}

impl MicrophoneSettings {
    /// What the effect chain should do.
    pub fn chain(&self) -> ChainSettings {
        ChainSettings {
            noise_removal: self.noise_removal.level(),
            room_echo_removal: self.room_echo_removal.level(),
            studio_voice: self.studio_voice.then(|| self.profile.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SpeakerSettings {
    /// Publish the virtual speakers, cleaned and sent on to `sink`.
    pub enabled: bool,
    /// The real output (a PipeWire node name); `None` is the system default.
    pub sink: Option<String>,
    pub noise_removal: Effect,
    pub echo_removal: Effect,
}

impl Default for SpeakerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            sink: None,
            noise_removal: Effect::new(true, 70),
            echo_removal: Effect::new(false, 50),
        }
    }
}

impl SpeakerSettings {
    pub fn chain(&self) -> ChainSettings {
        ChainSettings {
            noise_removal: self.noise_removal.level(),
            room_echo_removal: self.echo_removal.level(),
            studio_voice: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AudioSettings {
    pub microphone: MicrophoneSettings,
    pub speakers: SpeakerSettings,
}

impl AudioSettings {
    pub fn validate(&self) -> Result<(), AppError> {
        self.microphone
            .noise_removal
            .validate("microphone noise removal")?;
        self.microphone
            .room_echo_removal
            .validate("microphone room echo removal")?;
        self.speakers
            .noise_removal
            .validate("speaker noise removal")?;
        self.speakers
            .echo_removal
            .validate("speaker echo removal")?;
        for name in [&self.microphone.source, &self.speakers.sink]
            .into_iter()
            .flatten()
        {
            if name.is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
                return Err(AppError::InvalidSetting(
                    "an audio device name is empty, too long or has control characters".into(),
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_published_by_default_and_noise_removal_is_the_first_thing_on() {
        let settings = AudioSettings::default();
        assert!(!settings.microphone.enabled && !settings.speakers.enabled);
        assert_eq!(settings.microphone.chain().noise_removal, Some(0.8));
        assert_eq!(settings.microphone.chain().room_echo_removal, None);
        assert_eq!(settings.microphone.chain().studio_voice, None);
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn the_studio_voice_carries_the_microphone_profile() {
        let mut microphone = MicrophoneSettings {
            studio_voice: true,
            profile: Profile::Laptop,
            ..MicrophoneSettings::default()
        };
        assert_eq!(microphone.chain().studio_voice, Some(MicProfile::Laptop));
        microphone.profile = Profile::Desk;
        assert_eq!(microphone.chain().studio_voice, Some(MicProfile::Desk));
    }

    #[test]
    fn strengths_and_device_names_are_validated() {
        let mut settings = AudioSettings::default();
        settings.microphone.noise_removal.strength = 101;
        assert!(matches!(
            settings.validate(),
            Err(AppError::InvalidSetting(_))
        ));
        let mut settings = AudioSettings::default();
        settings.speakers.sink = Some(String::new());
        assert!(settings.validate().is_err());
        settings.speakers.sink = Some("bad\nname".into());
        assert!(settings.validate().is_err());
        settings.speakers.sink = Some("alsa_output.pci-0000_06_00.6.analog-stereo".into());
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn the_settings_round_trip_through_camel_case_json_with_defaults() {
        let parsed: AudioSettings =
            serde_json::from_str(r#"{"microphone":{"enabled":true,"studioVoice":true}}"#).unwrap();
        assert!(parsed.microphone.enabled && parsed.microphone.studio_voice);
        assert_eq!(parsed.microphone.noise_removal, Effect::new(true, 80));
        let json = serde_json::to_value(&parsed).unwrap();
        assert_eq!(json["microphone"]["roomEchoRemoval"]["strength"], 50);
    }

    #[test]
    fn an_effect_has_a_level_only_while_on_and_not_at_zero() {
        assert_eq!(Effect::new(true, 50).level(), Some(0.5));
        assert_eq!(Effect::new(false, 50).level(), None);
        assert_eq!(Effect::new(true, 0).level(), None);
    }
}
