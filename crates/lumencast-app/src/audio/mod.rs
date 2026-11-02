//! Audio: a virtual microphone and virtual speakers that clean what goes
//! through them (noise removal, room echo removal, studio voice), the sound
//! counterpart of the video effects and of the virtual camera.

pub mod devices;
pub mod engine;
pub mod io;
pub mod pump;
pub mod settings;
#[cfg(test)]
pub(crate) mod testing;

pub use engine::{AudioEngine, AudioPath, PathStatus};
pub use settings::AudioSettings;
