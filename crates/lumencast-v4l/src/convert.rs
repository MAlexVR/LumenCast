//! Pure conversions between V4L2/OS facts and core types. No device access.

use lumencast_core::error::CaptureError;
use lumencast_core::session::RawFrame;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Translates an OS error from opening or configuring `device`.
pub fn map_io_error(device: &str, error: &io::Error) -> CaptureError {
    const EPERM: i32 = 1;
    const ENOENT: i32 = 2;
    const EACCES: i32 = 13;
    const EBUSY: i32 = 16;
    const ENODEV: i32 = 19;
    match error.raw_os_error() {
        Some(ENOENT | ENODEV) => CaptureError::DeviceNotFound(device.to_owned()),
        Some(EBUSY) => CaptureError::DeviceBusy(device.to_owned()),
        Some(EACCES | EPERM) => CaptureError::PermissionDenied(device.to_owned()),
        _ => CaptureError::Acquisition(format!("{device}: {error}")),
    }
}

/// Builds a frame from a driver buffer. `bytes_used` bounds the payload because
/// the mapped buffer is usually larger than the frame it holds.
pub fn frame_from_buffer(
    buffer: &[u8],
    bytes_used: u32,
    seconds: i64,
    microseconds: i64,
) -> Result<RawFrame, CaptureError> {
    let timestamp_us = timestamp_us(seconds, microseconds)?;
    let used = usize::try_from(bytes_used).unwrap_or(usize::MAX);
    if used == 0 {
        return Err(CaptureError::Decode("empty packet".into()));
    }
    let data = buffer.get(..used).ok_or_else(|| {
        CaptureError::Decode(format!(
            "driver reported {used} bytes but the buffer holds {}",
            buffer.len()
        ))
    })?;
    Ok(RawFrame {
        timestamp_us,
        data: data.to_vec(),
    })
}

fn timestamp_us(seconds: i64, microseconds: i64) -> Result<u64, CaptureError> {
    let invalid = || {
        CaptureError::Acquisition(format!(
            "invalid capture timestamp {seconds}.{microseconds:06}"
        ))
    };
    if !(0..1_000_000).contains(&microseconds) {
        return Err(invalid());
    }
    u64::try_from(seconds)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000_000))
        .and_then(|base| base.checked_add(microseconds.unsigned_abs()))
        .ok_or_else(invalid)
}

/// Reads `vendor:product` (lowercase hex) of the USB device behind
/// `<sysfs>/video<index>`, or `None` if it is not a USB camera.
pub fn usb_id(sysfs_video4linux: &Path, index: usize) -> Option<String> {
    // `device` links to the USB interface; vendor and product sit on its parent.
    let usb_device = sysfs_video4linux.join(format!("video{index}/device/.."));
    let read = |name: &str| -> Option<String> {
        let text = fs::read_to_string(usb_device.join(name)).ok()?;
        Some(text.trim().to_lowercase()).filter(|value| !value.is_empty())
    };
    Some(format!("{}:{}", read("idVendor")?, read("idProduct")?))
}

/// Index of a `/dev/video<N>` node, or `None` for any other name.
pub fn device_index(path: &Path) -> Option<usize> {
    let digits = path.file_name()?.to_str()?.strip_prefix("video")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// `/dev/video<N>` nodes under `dev_dir`, ordered by index.
pub fn video_nodes(dev_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dev_dir) else {
        return Vec::new();
    };
    let mut nodes: Vec<(usize, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| Some((device_index(&entry.path())?, entry.path())))
        .collect();
    nodes.sort();
    nodes.into_iter().map(|(_, path)| path).collect()
}

/// Human name of `video<index>` from `<sysfs>/video<index>/name`.
pub fn device_name(sysfs_video4linux: &Path, index: usize) -> Option<String> {
    let text = fs::read_to_string(sysfs_video4linux.join(format!("video{index}/name"))).ok()?;
    Some(text.trim().to_owned()).filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    #[test]
    fn os_errors_map_to_typed_capture_errors() {
        let code = |raw| map_io_error("/dev/video0", &io::Error::from_raw_os_error(raw)).code();
        assert_eq!(code(2), "device_not_found");
        assert_eq!(code(19), "device_not_found");
        assert_eq!(code(16), "device_busy");
        assert_eq!(code(13), "permission_denied");
        assert_eq!(code(1), "permission_denied");
        assert_eq!(code(5), "acquisition_failed");
    }

    #[test]
    fn unknown_os_errors_keep_the_device_and_the_cause() {
        let error = map_io_error("/dev/video3", &io::Error::from_raw_os_error(5));
        let message = error.to_string();
        assert!(message.contains("/dev/video3"), "{message}");
        assert!(
            message.to_lowercase().contains("input/output error"),
            "{message}"
        );
    }

    #[test]
    fn frame_payload_is_bounded_by_bytes_used_and_time_is_microseconds() {
        let buffer = [1u8, 2, 3, 4, 5, 6];
        let frame = frame_from_buffer(&buffer, 4, 12, 345_678).unwrap();
        assert_eq!(frame.data, vec![1, 2, 3, 4]);
        assert_eq!(frame.timestamp_us, 12_345_678);
    }

    #[test]
    fn empty_and_inconsistent_buffers_are_decode_errors() {
        let buffer = [0u8; 4];
        assert_eq!(
            frame_from_buffer(&buffer, 0, 0, 0).unwrap_err().code(),
            "decode_failed"
        );
        assert_eq!(
            frame_from_buffer(&buffer, 9, 0, 0).unwrap_err().code(),
            "decode_failed"
        );
    }

    #[test]
    fn invalid_timestamps_are_acquisition_errors() {
        let buffer = [0u8; 4];
        for (seconds, microseconds) in [(-1, 0), (0, -1), (0, 1_000_000)] {
            let error = frame_from_buffer(&buffer, 4, seconds, microseconds).unwrap_err();
            assert_eq!(error.code(), "acquisition_failed");
        }
    }

    #[test]
    fn usb_id_is_read_from_the_parent_of_the_interface() {
        let root = tempfile::tempdir().unwrap();
        let usb = root.path().join("usb/1-1");
        let interface = usb.join("1-1:1.0");
        fs::create_dir_all(&interface).unwrap();
        fs::write(usb.join("idVendor"), "04E8\n").unwrap();
        fs::write(usb.join("idProduct"), "205d\n").unwrap();
        let class = root.path().join("class/video4linux");
        fs::create_dir_all(class.join("video2")).unwrap();
        symlink(&interface, class.join("video2/device")).unwrap();

        assert_eq!(usb_id(&class, 2).as_deref(), Some("04e8:205d"));
    }

    #[test]
    fn device_index_accepts_only_video_nodes() {
        assert_eq!(device_index(Path::new("/dev/video0")), Some(0));
        assert_eq!(device_index(Path::new("/dev/video12")), Some(12));
        assert_eq!(device_index(Path::new("/dev/video")), None);
        assert_eq!(device_index(Path::new("/dev/videoX")), None);
        assert_eq!(device_index(Path::new("/dev/video1x")), None);
        assert_eq!(device_index(Path::new("/dev/null")), None);
    }

    #[test]
    fn video_nodes_are_filtered_and_ordered_numerically() {
        let dev = tempfile::tempdir().unwrap();
        for name in ["video10", "video2", "video", "videoX", "media0", "video0"] {
            fs::write(dev.path().join(name), "").unwrap();
        }
        let names: Vec<String> = video_nodes(dev.path())
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["video0", "video2", "video10"]);
        assert!(video_nodes(&dev.path().join("missing")).is_empty());
    }

    #[test]
    fn device_name_is_trimmed_and_optional() {
        let sysfs = tempfile::tempdir().unwrap();
        fs::create_dir_all(sysfs.path().join("video4")).unwrap();
        fs::write(sysfs.path().join("video4/name"), "Samsung VG-STC3000\n").unwrap();
        assert_eq!(
            device_name(sysfs.path(), 4).as_deref(),
            Some("Samsung VG-STC3000")
        );
        assert_eq!(device_name(sysfs.path(), 5), None);
    }

    #[test]
    fn usb_id_is_none_for_non_usb_or_missing_devices() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("video0/device")).unwrap();
        assert_eq!(usb_id(root.path(), 0), None);
        assert_eq!(usb_id(root.path(), 7), None);
    }
}
