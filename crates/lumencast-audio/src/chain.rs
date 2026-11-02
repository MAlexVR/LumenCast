//! The microphone chain: noise removal, then room echo removal, then the
//! studio voice, each only while it is switched on.

use crate::dereverb::RoomEchoRemoval;
use crate::noise::NoiseRemoval;
use crate::voice::{MicProfile, StudioVoice};

/// What is switched on, and how strongly (0..=1).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ChainSettings {
    pub noise_removal: Option<f32>,
    pub room_echo_removal: Option<f32>,
    /// The studio voice, with the kind of microphone it is shaped for.
    pub studio_voice: Option<MicProfile>,
}

#[derive(Default)]
pub struct AudioChain {
    noise: Option<NoiseRemoval>,
    echo: Option<RoomEchoRemoval>,
    voice: Option<StudioVoice>,
    profile: Option<MicProfile>,
}

impl AudioChain {
    pub fn new(settings: ChainSettings) -> Self {
        let mut chain = Self::default();
        chain.update(settings);
        chain
    }

    /// Applies new settings; stages that stay on keep their state, so changing
    /// a strength does not make a click.
    pub fn update(&mut self, settings: ChainSettings) {
        match (settings.noise_removal, self.noise.as_mut()) {
            (Some(strength), Some(stage)) => stage.set_strength(strength),
            (Some(strength), None) => self.noise = Some(NoiseRemoval::new(strength)),
            (None, _) => self.noise = None,
        }
        match (settings.room_echo_removal, self.echo.as_mut()) {
            (Some(strength), Some(stage)) => stage.set_strength(strength),
            (Some(strength), None) => self.echo = Some(RoomEchoRemoval::new(strength)),
            (None, _) => self.echo = None,
        }
        match (settings.studio_voice, self.profile) {
            (Some(profile), current) if current != Some(profile) => {
                self.voice = Some(StudioVoice::with_profile(profile));
                self.profile = Some(profile);
            }
            (None, _) => {
                self.voice = None;
                self.profile = None;
            }
            _ => {}
        }
    }

    /// Whether any stage is on.
    pub fn is_active(&self) -> bool {
        self.noise.is_some() || self.echo.is_some() || self.voice.is_some()
    }

    /// Processes a block and returns as many samples.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mut samples = input.to_vec();
        if let Some(stage) = self.noise.as_mut() {
            samples = stage.process(&samples);
        }
        if let Some(stage) = self.echo.as_mut() {
            samples = stage.process(&samples);
        }
        if let Some(stage) = self.voice.as_mut() {
            samples = stage.process(&samples);
        }
        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn with_nothing_on_the_samples_pass_through_untouched() {
        let mut chain = AudioChain::new(ChainSettings::default());
        assert!(!chain.is_active());
        let input = vec![0.1, -0.2, 0.3];
        assert_eq!(chain.process(&input), input);
    }

    #[test]
    fn stages_come_and_go_with_the_settings_and_every_call_keeps_the_length() {
        let mut chain = AudioChain::new(ChainSettings {
            noise_removal: Some(0.8),
            room_echo_removal: Some(0.5),
            studio_voice: Some(MicProfile::Laptop),
        });
        assert!(chain.is_active());
        for len in [480, 100, 1000] {
            assert_eq!(chain.process(&vec![0.05; len]).len(), len);
        }
        chain.update(ChainSettings {
            noise_removal: None,
            room_echo_removal: Some(0.9),
            studio_voice: None,
        });
        assert_eq!(chain.process(&vec![0.05; 333]).len(), 333);
        chain.update(ChainSettings::default());
        assert!(!chain.is_active());
    }
}
