//! The studio voice: a clean-up equaliser, a compressor that evens out the
//! level and a limiter that keeps peaks under the ceiling. NVIDIA's
//! `studio_voice` enhances the voice with a neural model; this is the
//! classical broadcast chain.

use std::f32::consts::PI;

/// A second order filter section (Robert Bristow-Johnson's cookbook).
#[derive(Debug, Clone)]
pub struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}

impl Biquad {
    fn from(b0: f32, b1: f32, b2: f32, a0: f32, a1: f32, a2: f32) -> Self {
        Self {
            b: [b0 / a0, b1 / a0, b2 / a0],
            a: [a1 / a0, a2 / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }

    pub fn high_pass(sample_rate: f32, frequency: f32, q: f32) -> Self {
        let w = 2.0 * PI * frequency / sample_rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        Self::from(
            (1.0 + cos) / 2.0,
            -(1.0 + cos),
            (1.0 + cos) / 2.0,
            1.0 + alpha,
            -2.0 * cos,
            1.0 - alpha,
        )
    }

    pub fn peaking(sample_rate: f32, frequency: f32, q: f32, gain_db: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * frequency / sample_rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        Self::from(
            1.0 + alpha * a,
            -2.0 * cos,
            1.0 - alpha * a,
            1.0 + alpha / a,
            -2.0 * cos,
            1.0 - alpha / a,
        )
    }

    pub fn high_shelf(sample_rate: f32, frequency: f32, gain_db: f32) -> Self {
        let a = 10f32.powf(gain_db / 40.0);
        let w = 2.0 * PI * frequency / sample_rate;
        let (sin, cos) = w.sin_cos();
        let alpha = sin / 2.0 * 2f32.sqrt();
        let beta = 2.0 * a.sqrt() * alpha;
        Self::from(
            a * ((a + 1.0) + (a - 1.0) * cos + beta),
            -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
            a * ((a + 1.0) + (a - 1.0) * cos - beta),
            (a + 1.0) - (a - 1.0) * cos + beta,
            2.0 * ((a - 1.0) - (a + 1.0) * cos),
            (a + 1.0) - (a - 1.0) * cos - beta,
        )
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let output = self.b[0] * input + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [input, self.x[0]];
        self.y = [output, self.y[0]];
        output
    }
}

/// Below this level (dB) the compressor adds no make-up gain, above the next it adds all of it.
const MAKEUP_OFF_DB: f32 = -62.0;
const MAKEUP_ON_DB: f32 = -46.0;

fn db_to_linear(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Feed-forward compressor with a soft knee, working on the signal level in dB.
#[derive(Debug, Clone)]
struct Compressor {
    threshold_db: f32,
    ratio: f32,
    knee_db: f32,
    attack: f32,
    release: f32,
    makeup: f32,
    envelope_db: f32,
}

impl Compressor {
    fn new(sample_rate: f32) -> Self {
        let coefficient = |seconds: f32| (-1.0 / (seconds * sample_rate)).exp();
        Self {
            threshold_db: -22.0,
            ratio: 3.0,
            knee_db: 8.0,
            attack: coefficient(0.005),
            release: coefficient(0.120),
            makeup: db_to_linear(6.0),
            envelope_db: -100.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let level_db = 20.0 * input.abs().max(1e-6).log10();
        let coefficient = if level_db > self.envelope_db {
            self.attack
        } else {
            self.release
        };
        self.envelope_db = coefficient * self.envelope_db + (1.0 - coefficient) * level_db;
        let over = self.envelope_db - self.threshold_db;
        let reduction_db = if over <= -self.knee_db / 2.0 {
            0.0
        } else if over >= self.knee_db / 2.0 {
            over * (1.0 - 1.0 / self.ratio)
        } else {
            (1.0 - 1.0 / self.ratio) * (over + self.knee_db / 2.0).powi(2) / (2.0 * self.knee_db)
        };
        // The make-up gain fades out below the voice, so the room's noise floor is
        // not lifted along with the words.
        let fade =
            ((self.envelope_db - MAKEUP_OFF_DB) / (MAKEUP_ON_DB - MAKEUP_OFF_DB)).clamp(0.0, 1.0);
        let makeup = 1.0 + (self.makeup - 1.0) * fade;
        input * db_to_linear(-reduction_db) * makeup
    }
}

/// A limiter: an instant attack and a smooth release, so no sample passes the ceiling.
#[derive(Debug, Clone)]
struct Limiter {
    ceiling: f32,
    release: f32,
    gain: f32,
}

impl Limiter {
    fn new(sample_rate: f32) -> Self {
        Self {
            ceiling: db_to_linear(-1.0),
            release: (-1.0 / (0.050 * sample_rate)).exp(),
            gain: 1.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let needed = if input.abs() > self.ceiling {
            self.ceiling / input.abs()
        } else {
            1.0
        };
        self.gain = if needed < self.gain {
            needed
        } else {
            self.release * self.gain + (1.0 - self.release) * needed
        };
        (input * self.gain).clamp(-self.ceiling, self.ceiling)
    }
}

/// The kind of microphone the voice comes from. NVIDIA's `studio_voice` takes a
/// `micProfile` too ("mic profiles" models, downloaded later); here each profile
/// is a different equaliser curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MicProfile {
    /// A headset boom: close to the mouth, so less low end is needed.
    #[default]
    Headset,
    /// A laptop or webcam microphone: boxy and thin, so more is changed.
    Laptop,
    /// A desk or studio microphone: already good, so only a light touch.
    Desk,
}

pub struct StudioVoice {
    filters: Vec<Biquad>,
    compressor: Compressor,
    limiter: Limiter,
}

impl StudioVoice {
    pub fn new() -> Self {
        Self::with_profile(MicProfile::default())
    }

    pub fn with_profile(profile: MicProfile) -> Self {
        let rate = crate::SAMPLE_RATE as f32;
        // (high-pass Hz, boxiness dB at 260-300 Hz, presence dB, air dB)
        let (rumble, boxy, presence, air) = match profile {
            MicProfile::Headset => (90.0, -2.5, 3.0, 2.0),
            MicProfile::Laptop => (110.0, -4.0, 4.0, 3.0),
            MicProfile::Desk => (70.0, -1.5, 2.0, 1.5),
        };
        Self {
            filters: vec![
                // Rumble and handling noise below the voice.
                Biquad::high_pass(rate, rumble, 0.707),
                // Less boxiness in the low mids.
                Biquad::peaking(rate, 270.0, 0.9, boxy),
                // Presence, so words are clear.
                Biquad::peaking(rate, 3200.0, 0.9, presence),
                // A little air.
                Biquad::high_shelf(rate, 9000.0, air),
            ],
            compressor: Compressor::new(rate),
            limiter: Limiter::new(rate),
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        input
            .iter()
            .map(|sample| {
                let equalised = self.filters.iter_mut().fold(*sample, |s, f| f.process(s));
                self.limiter.process(self.compressor.process(equalised))
            })
            .collect()
    }
}

impl Default for StudioVoice {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::measure;

    fn sine(frequency: f32, amplitude: f32, seconds: f32) -> Vec<f32> {
        (0..(48_000.0 * seconds) as usize)
            .map(|n| amplitude * (2.0 * PI * frequency * n as f32 / 48_000.0).sin())
            .collect()
    }

    /// The gain of a single filter at a frequency, in dB.
    fn gain_at(mut filter: Biquad, frequency: f32) -> f32 {
        let input = sine(frequency, 0.1, 1.0);
        let out: Vec<f32> = input.iter().map(|s| filter.process(*s)).collect();
        measure(&out[24_000..]).rms_db - measure(&input[24_000..]).rms_db
    }

    #[test]
    fn the_filters_do_what_their_names_say() {
        let hp = || Biquad::high_pass(48_000.0, 90.0, 0.707);
        assert!(gain_at(hp(), 30.0) < -15.0, "rumble must go");
        assert!(
            gain_at(hp(), 1000.0).abs() < 0.3,
            "the voice band must stay"
        );
        let peak = || Biquad::peaking(48_000.0, 3200.0, 0.9, 3.0);
        assert!((gain_at(peak(), 3200.0) - 3.0).abs() < 0.3);
        assert!(gain_at(peak(), 200.0).abs() < 0.5);
        let shelf = || Biquad::high_shelf(48_000.0, 9000.0, 2.0);
        assert!((gain_at(shelf(), 18_000.0) - 2.0).abs() < 0.5);
        assert!(gain_at(shelf(), 500.0).abs() < 0.3);
    }

    #[test]
    fn quiet_and_loud_passages_end_up_closer_in_level() {
        let mut voice = StudioVoice::new();
        let quiet = voice.process(&sine(400.0, 0.02, 1.0));
        let loud = voice.process(&sine(400.0, 0.5, 1.0));
        let (q, l) = (
            measure(&quiet[30_000..]).rms_db,
            measure(&loud[30_000..]).rms_db,
        );
        let before =
            measure(&sine(400.0, 0.5, 0.5)).rms_db - measure(&sine(400.0, 0.02, 0.5)).rms_db;
        assert!(l - q < before - 6.0, "{} dB apart, was {before}", l - q);
    }

    #[test]
    fn nothing_goes_past_the_ceiling_even_for_a_huge_input() {
        let mut voice = StudioVoice::new();
        let out = voice.process(&sine(300.0, 4.0, 0.5));
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= db_to_linear(-1.0) + 1e-4, "peak {peak}");
    }

    #[test]
    fn silence_stays_silent_and_the_length_is_kept() {
        let mut voice = StudioVoice::new();
        let out = voice.process(&vec![0.0; 4800]);
        assert_eq!(out.len(), 4800);
        assert!(out.iter().all(|s| s.abs() < 1e-6));
    }

    #[test]
    fn the_noise_floor_is_not_lifted_but_a_voice_is() {
        let mut voice = StudioVoice::new();
        let hiss = sine(1000.0, 0.0008, 1.0); // about -62 dB
        let out = voice.process(&hiss);
        let change = measure(&out[30_000..]).rms_db - measure(&hiss[30_000..]).rms_db;
        assert!(change < 2.0, "the floor went up by {change} dB");
        let speech = sine(1000.0, 0.03, 1.0); // about -33 dB
        let out = voice.process(&speech);
        let lift = measure(&out[30_000..]).rms_db - measure(&speech[30_000..]).rms_db;
        assert!(lift > 3.0, "the voice was only lifted by {lift} dB");
    }

    #[test]
    fn each_profile_shapes_the_voice_differently() {
        let tone = sine(3200.0, 0.02, 1.0);
        let level = |profile| {
            let out = StudioVoice::with_profile(profile).process(&tone);
            measure(&out[30_000..]).rms_db
        };
        assert!(level(MicProfile::Laptop) > level(MicProfile::Headset));
        assert!(level(MicProfile::Headset) > level(MicProfile::Desk));
    }
}
