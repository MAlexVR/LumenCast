//! Audio effects for the microphone and the speakers. NVIDIA Broadcast offers
//! `noise_removal` (strength 0-1), `room_echo_removal` (strength 0-1) and
//! `studio_voice` for the microphone, and noise and echo removal for the
//! speakers, each with a neural model. Here noise removal runs RNNoise (a small
//! recurrent network, BSD-3), room echo removal is a spectral suppression of
//! the late reverberation, and the studio voice is an equaliser with a
//! compressor and a limiter. Everything works on mono 48 kHz samples in -1..1.

pub mod chain;
pub mod dereverb;
pub mod meter;
pub mod noise;
pub mod voice;

/// The sample rate every effect works at.
pub const SAMPLE_RATE: u32 = 48_000;
