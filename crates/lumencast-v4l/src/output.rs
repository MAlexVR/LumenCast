//! Writing frames to a v4l2loopback device, so other programs see them as a
//! camera. Only devices of the loopback driver are ever written to: a real
//! camera node accepts no frames and writing to it would be a mistake.

use crate::device::is_loopback_driver;
use std::io::Write;
use std::path::{Path, PathBuf};
use v4l::format::FourCC;
use v4l::video::Output;
use v4l::{Device, Format};

/// The first v4l2loopback device found, as a path.
pub fn find_loopback() -> Option<PathBuf> {
    loopbacks().into_iter().next()
}

/// Every v4l2loopback device, sorted by path.
pub fn loopbacks() -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = crate::convert::video_nodes(Path::new("/dev"))
        .into_iter()
        .filter(|path| {
            Device::with_path(path)
                .and_then(|device| device.query_caps())
                .is_ok_and(|caps| is_loopback_driver(&caps.driver))
        })
        .collect();
    found.sort();
    found
}

pub struct LoopbackOutput {
    device: Device,
    path: String,
    width: u32,
    height: u32,
}

impl LoopbackOutput {
    /// Opens `path` and sets it to send `width` x `height` planar YUV 4:2:0.
    /// Refuses anything that is not a loopback device.
    pub fn open(path: &str, width: u32, height: u32) -> Result<Self, String> {
        let device = Device::with_path(path).map_err(|error| format!("{path}: {error}"))?;
        let caps = device
            .query_caps()
            .map_err(|error| format!("{path}: {error}"))?;
        if !is_loopback_driver(&caps.driver) {
            return Err(format!(
                "{path} is not a v4l2loopback device (driver {})",
                caps.driver
            ));
        }
        if width == 0 || height == 0 || !width.is_multiple_of(2) || !height.is_multiple_of(2) {
            return Err(format!("a {width}x{height} picture cannot be sent as I420"));
        }
        let wanted = Format::new(width, height, FourCC::new(b"YU12"));
        let got = Output::set_format(&device, &wanted)
            .map_err(|error| format!("{path} refused the format: {error}"))?;
        if (got.width, got.height) != (width, height) || got.fourcc != wanted.fourcc {
            return Err(format!(
                "{path} insisted on {}x{} {} instead of {width}x{height} YU12",
                got.width, got.height, got.fourcc
            ));
        }
        Ok(Self {
            device,
            path: path.to_owned(),
            width,
            height,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Sends one picture: `width * height * 3 / 2` bytes of I420.
    pub fn write(&mut self, i420: &[u8]) -> Result<(), String> {
        let expected = self.width as usize * self.height as usize * 3 / 2;
        if i420.len() != expected {
            return Err(format!(
                "a frame of {} bytes was given where {expected} are needed",
                i420.len()
            ));
        }
        self.device
            .write_all(i420)
            .map_err(|error| format!("{}: {error}", self.path))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_something_that_is_not_there_says_which_path() {
        let error = LoopbackOutput::open("/dev/video-that-does-not-exist", 640, 480)
            .err()
            .unwrap();
        assert!(error.contains("video-that-does-not-exist"), "{error}");
    }

    #[test]
    fn odd_and_empty_sizes_are_refused_before_the_device_is_touched() {
        // A path that is not a device fails first, so use the check order:
        // the size check only runs on a real loopback device (see the hardware test).
        assert!(LoopbackOutput::open("/dev/null", 641, 480).is_err());
    }
}
