//! Sending the processed video to another program as a virtual camera.

use lumencast_fx::convert::rgb_to_i420;
use lumencast_fx::frame::Frame;
use lumencast_v4l::output::{find_loopback, LoopbackOutput};
use serde::Serialize;

/// Where the processed frames go. One frame size per output; a new size
/// needs a new output.
pub trait FrameOutput: Send {
    fn send(&mut self, frame: &Frame) -> Result<(), String>;
    /// A name for the destination, for the UI.
    fn device(&self) -> String;
}

/// Opens outputs on the worker thread, when the first frame has to go out.
pub trait OutputFactory: Send + Sync + 'static {
    /// An output for pictures of `width` x `height` (both even).
    fn open(&self, width: u32, height: u32) -> Result<Box<dyn FrameOutput>, String>;
}

/// Used where nothing may be sent (tests, builds without a virtual camera).
pub struct NoOutput;

impl OutputFactory for NoOutput {
    fn open(&self, _: u32, _: u32) -> Result<Box<dyn FrameOutput>, String> {
        Err("no virtual camera is configured".into())
    }
}

/// How the virtual camera is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum VirtualCameraStatus {
    Off,
    Active {
        device: String,
        width: u32,
        height: u32,
    },
    Unavailable {
        reason: String,
    },
}

/// The v4l2loopback device of this machine.
pub struct LoopbackFactory;

struct LoopbackFrames(LoopbackOutput);

impl FrameOutput for LoopbackFrames {
    fn send(&mut self, frame: &Frame) -> Result<(), String> {
        let (width, height, i420) = rgb_to_i420(frame);
        if (width, height) != self.0.size() {
            return Err(format!(
                "the frame is {width}x{height} but the virtual camera was opened at {}x{}",
                self.0.size().0,
                self.0.size().1
            ));
        }
        self.0.write(&i420)
    }

    fn device(&self) -> String {
        self.0.path().to_owned()
    }
}

impl OutputFactory for LoopbackFactory {
    fn open(&self, width: u32, height: u32) -> Result<Box<dyn FrameOutput>, String> {
        let path = find_loopback().ok_or_else(|| {
            "no virtual camera device was found; load the v4l2loopback module".to_owned()
        })?;
        let output = LoopbackOutput::open(&path.display().to_string(), width, height)?;
        Ok(Box::new(LoopbackFrames(output)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_sent_where_no_output_is_configured() {
        assert!(NoOutput.open(640, 360).is_err());
    }

    #[test]
    fn the_status_serializes_with_a_state_tag() {
        let active = VirtualCameraStatus::Active {
            device: "/dev/video10".into(),
            width: 1280,
            height: 720,
        };
        let json = serde_json::to_value(active).unwrap();
        assert_eq!(json["state"], "active");
        assert_eq!(json["device"], "/dev/video10");
        assert_eq!(
            serde_json::to_value(VirtualCameraStatus::Off).unwrap()["state"],
            "off"
        );
        let problem =
            serde_json::to_value(VirtualCameraStatus::Unavailable { reason: "x".into() }).unwrap();
        assert_eq!(problem["reason"], "x");
    }
}
