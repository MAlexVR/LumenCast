//! Application layer: turns UI intents into capture sessions and capture
//! progress into events. No Tauri types here, so it is testable offline.

pub mod audio;
pub mod autostart;
pub mod backdrop;
pub mod core;
pub mod error;
pub mod events;
pub mod output;
pub mod preview;
pub mod processing;
pub mod profile;
pub mod settings;
