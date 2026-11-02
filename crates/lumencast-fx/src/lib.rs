//! Deterministic frame stages. Pure functions over RGB frames: no I/O, no
//! models, no GPU. Semantics of manual framing follow the legacy Blucast
//! server so existing zoom and pan settings keep their meaning.

pub mod background;
pub mod codec;
pub mod convert;
pub mod denoise;
pub mod error;
pub mod frame;
pub mod keylight;
pub mod lightprobe;
pub mod matte;
pub mod params;
pub mod pipeline;
pub mod reframe;
pub mod resample;
pub mod stages;
