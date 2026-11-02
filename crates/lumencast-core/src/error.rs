//! Typed capture failures with stable codes for the UI boundary.

use crate::mode::ModeError;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("camera device not found: {0}")]
    DeviceNotFound(String),
    #[error("camera device is busy: {0}")]
    DeviceBusy(String),
    #[error("permission denied opening camera device: {0}")]
    PermissionDenied(String),
    #[error(transparent)]
    Mode(#[from] ModeError),
    #[error("frame acquisition failed: {0}")]
    Acquisition(String),
    #[error("frame decode failed: {0}")]
    Decode(String),
    #[error("capture cancelled")]
    Cancelled,
}

/// `{code, message}` as sent over the Tauri boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorPayload {
    pub code: &'static str,
    pub message: String,
}

impl CaptureError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::DeviceNotFound(_) => "device_not_found",
            Self::DeviceBusy(_) => "device_busy",
            Self::PermissionDenied(_) => "permission_denied",
            Self::Mode(ModeError::Unsupported { .. }) => "mode_unsupported",
            Self::Mode(_) => "mode_invalid",
            Self::Acquisition(_) => "acquisition_failed",
            Self::Decode(_) => "decode_failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn payload(&self) -> ErrorPayload {
        ErrorPayload {
            code: self.code(),
            message: self.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::{CaptureMode, FrameInterval, FrameSize, PixelFormat};

    fn unsupported() -> ModeError {
        let mode = CaptureMode {
            format: PixelFormat::parse("MJPG").unwrap(),
            size: FrameSize::new(1280, 720).unwrap(),
            interval: FrameInterval::new(1, 60).unwrap(),
        };
        ModeError::Unsupported {
            requested: mode,
            alternatives: vec![],
        }
    }

    #[test]
    fn every_variant_has_a_stable_code() {
        let cases = [
            (CaptureError::DeviceNotFound("x".into()), "device_not_found"),
            (CaptureError::DeviceBusy("x".into()), "device_busy"),
            (
                CaptureError::PermissionDenied("x".into()),
                "permission_denied",
            ),
            (CaptureError::Mode(unsupported()), "mode_unsupported"),
            (
                CaptureError::Mode(ModeError::InvalidSize {
                    width: 0,
                    height: 1,
                }),
                "mode_invalid",
            ),
            (CaptureError::Acquisition("x".into()), "acquisition_failed"),
            (CaptureError::Decode("x".into()), "decode_failed"),
            (CaptureError::Cancelled, "cancelled"),
        ];
        for (error, code) in cases {
            assert_eq!(error.code(), code);
        }
    }

    #[test]
    fn payload_serializes_as_code_and_message() {
        let json =
            serde_json::to_value(CaptureError::DeviceBusy("/dev/video0".into()).payload()).unwrap();
        assert_eq!(json["code"], "device_busy");
        assert_eq!(json["message"], "camera device is busy: /dev/video0");
    }

    #[test]
    fn mode_errors_convert_into_capture_errors() {
        let error: CaptureError = unsupported().into();
        assert!(matches!(error, CaptureError::Mode(_)));
    }
}
