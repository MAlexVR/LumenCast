//! Errors the UI can see, with stable codes.

use lumencast_core::error::{CaptureError, ErrorPayload};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error("no camera with index {0}")]
    UnknownCamera(usize),
    #[error("a capture is already running")]
    AlreadyRunning,
    #[error("no capture is running")]
    NotRunning,
    #[error("invalid setting: {0}")]
    InvalidSetting(String),
    #[error("{0} frames cannot be shown in the preview yet")]
    FormatNotPreviewable(String),
    #[error("the setting was applied but could not be saved: {0}")]
    SettingsNotSaved(String),
    #[error("autostart could not be changed: {0}")]
    AutostartFailed(String),
    #[error("audio is not available: {0}")]
    AudioUnavailable(String),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Capture(error) => error.code(),
            Self::UnknownCamera(_) => "device_not_found",
            Self::AlreadyRunning => "capture_already_running",
            Self::NotRunning => "capture_not_running",
            Self::InvalidSetting(_) => "invalid_setting",
            Self::FormatNotPreviewable(_) => "format_not_previewable",
            Self::SettingsNotSaved(_) => "settings_not_saved",
            Self::AutostartFailed(_) => "autostart_failed",
            Self::AudioUnavailable(_) => "audio_unavailable",
        }
    }

    pub fn payload(&self) -> ErrorPayload {
        ErrorPayload {
            code: self.code(),
            message: self.to_string(),
        }
    }
}
