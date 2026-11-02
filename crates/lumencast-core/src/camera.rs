//! Camera catalog facts shared by adapters and the application layer.

use crate::cancel::CancelFlag;
use crate::error::CaptureError;
use crate::mode::{CameraCapabilities, CaptureMode};
use crate::session::{SessionConfig, SessionObserver, SessionOutcome};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    pub path: String,
    pub index: usize,
    pub name: Option<String>,
    /// `vendor:product` for USB cameras, stable across `/dev/video` renumbering.
    pub usb_id: Option<String>,
}

/// A mode or range that was enumerated but could not become a capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedMode {
    pub format: String,
    pub size: Option<(u32, u32)>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CameraProbe {
    pub capabilities: CameraCapabilities,
    pub skipped: Vec<SkippedMode>,
}

/// Everything the application needs from the platform's cameras.
pub trait CameraCatalog: Send + Sync + 'static {
    fn list(&self) -> Vec<CameraInfo>;

    fn probe(&self, device_path: &str) -> Result<CameraProbe, CaptureError>;

    /// Opens the device and runs one capture session on the calling thread.
    fn run(
        &self,
        device_path: &str,
        request: &CaptureMode,
        config: SessionConfig,
        cancel: &CancelFlag,
        observer: &mut dyn SessionObserver,
    ) -> Result<SessionOutcome, CaptureError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_info_serializes_in_camel_case() {
        let info = CameraInfo {
            path: "/dev/video2".into(),
            index: 2,
            name: Some("SAMSUNG VD USB HDCAM".into()),
            usb_id: Some("04e8:205d".into()),
        };
        let json = serde_json::to_value(info).unwrap();
        assert_eq!(json["usbId"], "04e8:205d");
        assert_eq!(json["index"], 2);
    }

    #[test]
    fn skipped_mode_keeps_its_reason_and_optional_size() {
        let skipped = SkippedMode {
            format: "MJPG".into(),
            size: Some((1280, 720)),
            reason: "stepwise frame intervals are not enumerated".into(),
        };
        let json = serde_json::to_value(skipped).unwrap();
        assert_eq!(json["size"], serde_json::json!([1280, 720]));
        assert!(json["reason"].as_str().unwrap().contains("stepwise"));
    }
}
