//! Noise removal with RNNoise (through the `nnnoiseless` port). It works on
//! frames of 10 ms and is trained on speech, so steady noise, fans and
//! keyboards are removed and the voice is kept.

use nnnoiseless::DenoiseState;
use std::collections::VecDeque;

/// RNNoise takes 480 samples at a time.
pub const FRAME: usize = DenoiseState::FRAME_SIZE;
/// RNNoise works with samples in the 16-bit range.
const SCALE: f32 = 32768.0;

pub struct NoiseRemoval {
    state: Box<DenoiseState<'static>>,
    /// 0 leaves the voice as it is, 1 removes as much noise as the model can.
    strength: f32,
    pending: Vec<f32>,
    ready: VecDeque<f32>,
    first_frame: bool,
    /// The gain of the gate that closes on frames the model takes for noise.
    gate: f32,
}

impl NoiseRemoval {
    pub fn new(strength: f32) -> Self {
        Self {
            state: DenoiseState::new(),
            strength: strength.clamp(0.0, 1.0),
            pending: Vec::with_capacity(FRAME * 2),
            // A frame of silence up front, so every call returns as many
            // samples as it was given (the cost is 10 ms of delay).
            ready: std::iter::repeat_n(0.0, FRAME).collect(),
            first_frame: true,
            gate: 1.0,
        }
    }

    pub fn set_strength(&mut self, strength: f32) {
        self.strength = strength.clamp(0.0, 1.0);
    }

    /// Cleans `input` and returns as many samples.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        self.pending.extend_from_slice(input);
        while self.pending.len() >= FRAME {
            let dry: Vec<f32> = self.pending.drain(..FRAME).collect();
            let scaled: Vec<f32> = dry.iter().map(|s| s * SCALE).collect();
            let mut wet = vec![0.0f32; FRAME];
            let speech = self.state.process_frame(&mut wet, &scaled);
            // RNNoise says how likely the frame is speech; when it is not, the
            // gate closes a little more than the network alone does (up to 24 dB at
            // full strength), and it opens again at once when a voice returns.
            let closed = 10f32.powf(-24.0 * self.strength / 20.0);
            let target = if speech < 0.4 { closed } else { 1.0 };
            let from = self.gate;
            self.gate = if target > from {
                target
            } else {
                from * 0.6 + target * 0.4
            };
            if self.first_frame {
                // The model's first output is a fade-in artefact.
                self.first_frame = false;
                wet.fill(0.0);
            }
            for (n, (dry, wet)) in dry.iter().zip(&wet).enumerate() {
                // The gate moves smoothly across the frame, so it never clicks.
                let gate = from + (self.gate - from) * (n + 1) as f32 / FRAME as f32;
                let wet = wet / SCALE * gate;
                self.ready
                    .push_back(dry * (1.0 - self.strength) + wet * self.strength);
            }
        }
        (0..input.len())
            .map(|_| self.ready.pop_front().unwrap_or(0.0))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meter::measure;

    /// Deterministic white noise at the given amplitude.
    fn noise(len: usize, amplitude: f32, seed: u32) -> Vec<f32> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * amplitude
            })
            .collect()
    }

    #[test]
    fn steady_noise_is_removed() {
        let mut removal = NoiseRemoval::new(1.0);
        let input = noise(48_000 * 2, 0.05, 7);
        let mut out = Vec::new();
        for block in input.chunks(480) {
            out.extend(removal.process(block));
        }
        // Skip the model's warm up.
        let (before, after) = (measure(&input[24_000..]), measure(&out[24_000..]));
        assert!(
            after.rms_db < before.rms_db - 9.0,
            "{} dB before, {} dB after",
            before.rms_db,
            after.rms_db
        );
    }

    #[test]
    fn strength_zero_passes_the_signal_through_with_the_fixed_delay() {
        let mut removal = NoiseRemoval::new(0.0);
        let input = noise(4800, 0.1, 3);
        let mut out = Vec::new();
        for block in input.chunks(480) {
            out.extend(removal.process(block));
        }
        assert_eq!(out.len(), input.len());
        for (a, b) in input[..input.len() - FRAME].iter().zip(&out[FRAME..]) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn any_block_size_returns_as_many_samples_as_it_was_given() {
        let mut removal = NoiseRemoval::new(0.5);
        for len in [1, 7, 100, 479, 480, 481, 2000] {
            assert_eq!(removal.process(&noise(len, 0.1, len as u32)).len(), len);
        }
    }

    #[test]
    fn the_strength_changes_how_much_is_removed() {
        let run = |strength: f32| {
            let mut removal = NoiseRemoval::new(strength);
            let input = noise(48_000, 0.05, 11);
            let mut out = Vec::new();
            for block in input.chunks(480) {
                out.extend(removal.process(block));
            }
            measure(&out[24_000..]).rms_db
        };
        assert!(run(1.0) < run(0.4) - 3.0);
    }
}
