//! Room echo removal: suppression of the late reverberation in the spectral
//! domain. A reverberant room keeps the sound going after the voice has
//! stopped, with an energy that falls off at a known rate (the reverberation
//! time). For every frequency the energy that the room would still be adding
//! is estimated from what was heard a little earlier, and taken out with a
//! Wiener-style gain. NVIDIA's `room_echo_removal` does this with a neural
//! model; this is the classical method (Lebart, Boucher and Denbigh).

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use std::collections::VecDeque;
use std::sync::Arc;

const N: usize = 1024;
const HOP: usize = 256;
const BINS: usize = N / 2 + 1;
/// The part of the reverberation that is "early" and sounds like the voice
/// itself: what is heard within this time is kept.
const EARLY_SECONDS: f32 = 0.05;
/// How the room is assumed to ring: the time its energy takes to fall 60 dB.
const REVERB_SECONDS: f32 = 0.5;
/// How much of the previous averaged power each bin keeps from frame to frame.
const POWER_MEMORY: f32 = 0.7;

pub struct RoomEchoRemoval {
    strength: f32,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    window: Vec<f32>,
    input: Vec<f32>,
    /// Overlap-add output not yet handed out.
    output: VecDeque<f32>,
    /// Power spectra of the frames so far, newest last.
    history: VecDeque<Vec<f32>>,
    /// The gain used for the previous frame, to smooth the next one.
    previous_gain: Vec<f32>,
    /// The power of each bin averaged over time (a single frame of a noisy tail is too random to compare).
    averaged: Vec<f32>,
    /// Frames of delay between "heard" and "late".
    early_frames: usize,
    decay: f32,
    /// How much of the late estimate is taken away: base plus slope x strength.
    over: (f32, f32),
    /// The quietest the gain may get, in dB below zero: base plus slope x strength.
    floor_db: (f32, f32),
}

fn hann(n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos())
        .collect()
}

impl RoomEchoRemoval {
    pub fn new(strength: f32) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let early_frames =
            ((EARLY_SECONDS * crate::SAMPLE_RATE as f32) / HOP as f32).round() as usize;
        // Energy falls by 60 dB (a factor of 1e6) over REVERB_SECONDS.
        let per_second = 10f32.powf(-6.0 / REVERB_SECONDS);
        let decay = per_second.powf(early_frames as f32 * HOP as f32 / crate::SAMPLE_RATE as f32);
        Self {
            strength: strength.clamp(0.0, 1.0),
            forward: planner.plan_fft_forward(N),
            inverse: planner.plan_fft_inverse(N),
            window: hann(N),
            // The first frame is complete once N samples have arrived; start with
            // N - HOP samples of silence so the output lines up with the input.
            input: vec![0.0; N - HOP],
            output: std::iter::repeat_n(0.0, N - HOP).collect(),
            history: VecDeque::new(),
            previous_gain: vec![1.0; BINS],
            averaged: vec![0.0; BINS],
            early_frames: early_frames.max(1),
            decay,
            over: (0.4, 1.2),
            floor_db: (6.0, 10.0),
        }
    }

    /// For tuning in tests: the over-subtraction and the floor as (base, slope).
    #[cfg(test)]
    fn tuned(strength: f32, over: (f32, f32), floor_db: (f32, f32)) -> Self {
        let mut removal = Self::new(strength);
        removal.over = over;
        removal.floor_db = floor_db;
        removal
    }

    pub fn set_strength(&mut self, strength: f32) {
        self.strength = strength.clamp(0.0, 1.0);
    }

    /// Cleans `input` and returns as many samples.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.input.extend_from_slice(input);
        while self.input.len() >= N {
            let frame: Vec<f32> = self.input[..N].to_vec();
            self.input.drain(..HOP);
            self.frame(&frame);
        }
        (0..input.len())
            .map(|_| self.output.pop_front().unwrap_or(0.0))
            .collect()
    }

    fn frame(&mut self, samples: &[f32]) {
        let mut windowed: Vec<f32> = samples
            .iter()
            .zip(&self.window)
            .map(|(s, w)| s * w)
            .collect();
        let mut spectrum = vec![Complex::new(0.0, 0.0); BINS];
        if self.forward.process(&mut windowed, &mut spectrum).is_err() {
            return;
        }
        let instant: Vec<f32> = spectrum.iter().map(|c| c.norm_sqr()).collect();
        for (average, now) in self.averaged.iter_mut().zip(&instant) {
            *average = POWER_MEMORY * *average + (1.0 - POWER_MEMORY) * now;
        }
        let power = self.averaged.clone();

        // What the room is still adding now: what was heard `early_frames` ago, decayed.
        self.history.push_back(power.clone());
        while self.history.len() > self.early_frames + 1 {
            self.history.pop_front();
        }
        let late: Vec<f32> = if self.history.len() > self.early_frames {
            self.history[0].iter().map(|p| p * self.decay).collect()
        } else {
            vec![0.0; BINS]
        };

        // Stronger settings take more of the estimate away and allow a lower floor.
        let over_subtraction = self.over.0 + self.over.1 * self.strength;
        let floor = 10f32.powf(-(self.floor_db.0 + self.floor_db.1 * self.strength) / 20.0);
        for (k, bin) in spectrum.iter_mut().enumerate() {
            let wanted = if power[k] > 1e-12 {
                (1.0 - over_subtraction * late[k] / power[k]).max(floor)
            } else {
                1.0
            };
            // Quick to let sound through, slow to close, so words do not get chopped.
            let smoothing = if wanted > self.previous_gain[k] {
                0.4
            } else {
                0.8
            };
            let gain = smoothing * self.previous_gain[k] + (1.0 - smoothing) * wanted;
            self.previous_gain[k] = gain;
            *bin *= gain;
        }
        let mut time = vec![0.0f32; N];
        if self.inverse.process(&mut spectrum, &mut time).is_err() {
            return;
        }
        // Hann at 75% overlap sums to 2 (analysis x synthesis), and the inverse
        // transform is not normalised.
        let scale = 1.0 / (N as f32 * 1.5);
        while self.output.len() < N - HOP {
            self.output.push_back(0.0);
        }
        let base = self.output.len() - (N - HOP);
        for (i, (t, w)) in time.iter().zip(&self.window).enumerate() {
            let value = t * w * scale;
            if base + i < self.output.len() {
                self.output[base + i] += value;
            } else {
                self.output.push_back(value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::measure;

    /// Bursts of a voiced-like tone, 0.4 s on and 0.6 s off, at 48 kHz.
    fn bursts(seconds: usize) -> Vec<f32> {
        (0..48_000 * seconds)
            .map(|n| {
                let t = n as f32 / 48_000.0;
                let phase = t % 1.0;
                if phase < 0.4 {
                    let f0 = 140.0;
                    (1..=8)
                        .map(|h| (2.0 * std::f32::consts::PI * f0 * h as f32 * t).sin() / h as f32)
                        .sum::<f32>()
                        * 0.2
                } else {
                    0.0
                }
            })
            .collect()
    }

    /// The same convolved with a room: an exponentially decaying noise.
    fn in_a_room(dry: &[f32], seconds: f32) -> Vec<f32> {
        let length = (48_000.0 * seconds) as usize;
        let mut state = 99u32;
        let impulse: Vec<f32> = (0..length)
            .map(|n| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                noise * 10f32.powf(-3.0 * n as f32 / length as f32)
            })
            .collect();
        let mut out = vec![0.0f32; dry.len()];
        // Sparse convolution: only every 8th tap, enough to build a tail.
        for (i, x) in dry.iter().enumerate().filter(|(_, x)| x.abs() > 1e-6) {
            for (j, h) in impulse.iter().enumerate().step_by(8) {
                if i + j < out.len() {
                    out[i + j] += x * h * 0.5;
                }
            }
        }
        for (o, d) in out.iter_mut().zip(dry) {
            *o += d;
        }
        out
    }

    fn run(strength: f32, input: &[f32]) -> Vec<f32> {
        let mut removal = RoomEchoRemoval::new(strength);
        let mut out = Vec::new();
        for block in input.chunks(480) {
            out.extend(removal.process(block));
        }
        out
    }

    /// The level in the quiet part of every second, after the tail has begun.
    fn tail_level(signal: &[f32]) -> f32 {
        let mut tails = Vec::new();
        for second in 2..5 {
            let start = second * 48_000 + 24_000;
            tails.extend_from_slice(&signal[start..start + 12_000]);
        }
        measure(&tails).rms_db
    }

    #[test]
    fn the_tail_of_a_reverberant_room_is_reduced_and_the_voice_is_kept() {
        let dry = bursts(6);
        let wet = in_a_room(&dry, 0.5);
        let cleaned = run(1.0, &wet);
        assert_eq!(cleaned.len(), wet.len());
        let (before, after) = (tail_level(&wet), tail_level(&cleaned));
        assert!(
            after < before - 6.0,
            "tail {before} dB before, {after} dB after"
        );
        // The voice itself (the first 0.2 s of each burst) keeps its level.
        let voice = |s: &[f32]| measure(&s[3 * 48_000 + 4800..3 * 48_000 + 14_400]).rms_db;
        assert!(
            (voice(&cleaned) - voice(&wet)).abs() < 5.0,
            "voice {} dB before, {} dB after",
            voice(&wet),
            voice(&cleaned)
        );
    }

    #[test]
    fn a_stronger_setting_removes_more_of_the_tail() {
        let wet = in_a_room(&bursts(6), 0.5);
        assert!(tail_level(&run(1.0, &wet)) < tail_level(&run(0.2, &wet)) - 1.0);
    }

    #[test]
    fn a_dry_signal_is_left_nearly_alone_and_blocks_of_any_size_work() {
        let dry = bursts(3);
        let cleaned = run(0.5, &dry);
        let burst = |s: &[f32]| measure(&s[48_000 + 4800..48_000 + 14_400]).rms_db;
        // A perfectly steady tone is the worst case for this method (it looks like
        // its own reverberation), and at half strength it still loses under 3 dB.
        assert!((burst(&cleaned) - burst(&dry)).abs() < 3.0);
        let mut removal = RoomEchoRemoval::new(0.5);
        for len in [1, 5, 255, 256, 257, 1000, 3000] {
            assert_eq!(removal.process(&vec![0.01; len]).len(), len);
        }
    }

    #[test]
    #[ignore = "prints a table used to choose the constants"]
    fn tuning_table() {
        let dry = bursts(6);
        let wet = in_a_room(&dry, 0.5);
        let voice = |s: &[f32]| measure(&s[3 * 48_000 + 4800..3 * 48_000 + 14_400]).rms_db;
        println!(
            "tail before {:.1} dB, voice before {:.1} dB",
            tail_level(&wet),
            voice(&wet)
        );
        for over in [0.6f32, 0.9, 1.2, 1.5, 2.0] {
            for floor in [8.0f32, 14.0, 20.0] {
                let mut removal = RoomEchoRemoval::tuned(1.0, (over, 0.0), (floor, 0.0));
                let mut out = Vec::new();
                for block in wet.chunks(480) {
                    out.extend(removal.process(block));
                }
                let mut removal = RoomEchoRemoval::tuned(1.0, (over, 0.0), (floor, 0.0));
                let mut dry_out = Vec::new();
                for block in dry.chunks(480) {
                    dry_out.extend(removal.process(block));
                }
                let burst = |s: &[f32]| measure(&s[48_000 + 4800..48_000 + 14_400]).rms_db;
                println!(
                    "over {over:.1} floor {floor:>4.0} dB: tail {:+.1} dB, voice in the room {:+.1} dB, voice dry {:+.1} dB",
                    tail_level(&out) - tail_level(&wet),
                    voice(&out) - voice(&wet),
                    burst(&dry_out) - burst(&dry)
                );
            }
        }
    }
}
