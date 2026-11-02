//! V4L2 device access. This is the only module that talks to the kernel; the
//! logic around it lives in the pure modules and is unit tested. It is
//! verified on real hardware by the ignored host acceptance tests.

use crate::convert::{device_name, frame_from_buffer, map_io_error, usb_id, video_nodes};
use crate::jpeg::validate_mjpeg;
use crate::probe::{map_probe, FormatProbe, IntervalProbe, SizeProbe, SizesProbe};
use lumencast_core::camera::{CameraCatalog, CameraInfo, CameraProbe};
use lumencast_core::cancel::CancelFlag;
use lumencast_core::error::CaptureError;
use lumencast_core::h264::{has_keyframe, KeyframeGate, Verdict, MAX_RESTARTS};
use lumencast_core::mode::{
    CameraCapabilities, CaptureMode, FrameInterval, FrameSize, PixelFormat,
};
use lumencast_core::session::{
    run_capture_observed, CameraPort, EndReason, FrameSource, RawFrame, SessionConfig,
    SessionObserver, SessionOutcome,
};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use v4l::buffer::Type;
use v4l::device::Handle;
use v4l::frameinterval::FrameIntervalEnum;
use v4l::framesize::FrameSizeEnum;
use v4l::io::mmap::Stream;
use v4l::io::traits::CaptureStream;
use v4l::video::capture::Parameters as CaptureParameters;
use v4l::video::Capture;
use v4l::{Device, Format, FourCC, Fraction};

const DEV_DIR: &str = "/dev";
const SYSFS_VIDEO4LINUX: &str = "/sys/class/video4linux";
const MJPEG: [u8; 4] = *b"MJPG";
const H264: [u8; 4] = *b"H264";
/// The reason a session ends when an H264 stream's keyframe did not come: the
/// camera is closed and opened again, which is what makes it send one. The
/// stream is never started again on an open device: that has hung cameras.
const KEYFRAME_MISSING: &str = "the H264 stream did not start with a keyframe";
/// How long the camera is left alone between closing and opening it again.
const REOPEN_PAUSE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy)]
pub struct V4lOptions {
    /// How long one `next_frame` call waits before reporting an idle poll.
    pub poll_timeout: Duration,
    /// How long the first frame may take after the stream starts. Cameras
    /// adjusting exposure can need a second or more; running out of time here
    /// is fatal because the `v4l` stream cannot be resumed after a timeout.
    pub first_frame_timeout: Duration,
    /// Consecutive idle polls after which the stream counts as stalled.
    pub stall_limit: u32,
    pub buffer_count: u32,
}

impl Default for V4lOptions {
    fn default() -> Self {
        Self {
            poll_timeout: Duration::from_millis(250),
            first_frame_timeout: Duration::from_secs(4),
            stall_limit: 20,
            buffer_count: 4,
        }
    }
}

/// The v4l2loopback kernel module, whose devices carry what a program writes
/// into them; listing one as a camera would feed our own output back as input.
pub fn is_loopback_driver(driver: &str) -> bool {
    driver.to_ascii_lowercase().contains("loopback")
}

/// Capture nodes only: metadata nodes of the same camera enumerate no formats.
pub fn list_cameras() -> Vec<CameraInfo> {
    video_nodes(Path::new(DEV_DIR))
        .into_iter()
        .filter_map(|path| {
            let index = crate::convert::device_index(&path)?;
            let device = Device::with_path(&path).ok()?;
            // A virtual camera we (or Blucast) feed is an output, not a source.
            if device
                .query_caps()
                .is_ok_and(|caps| is_loopback_driver(&caps.driver))
            {
                return None;
            }
            let formats = device.enum_formats().ok()?;
            if formats.is_empty() {
                return None;
            }
            let sysfs = Path::new(SYSFS_VIDEO4LINUX);
            Some(CameraInfo {
                path: path.display().to_string(),
                index,
                name: device_name(sysfs, index),
                usb_id: usb_id(sysfs, index),
            })
        })
        .collect()
}

pub struct V4lCamera {
    /// Times the stream of an H264 camera may still be reopened for a missing keyframe.
    keyframe_restarts: u32,
    path: String,
    device: Device,
    options: V4lOptions,
}

impl V4lCamera {
    pub fn open_path(path: &str, options: V4lOptions) -> Result<Self, CaptureError> {
        let device = Device::with_path(path).map_err(|error| map_io_error(path, &error))?;
        Ok(Self {
            keyframe_restarts: MAX_RESTARTS,
            path: path.to_owned(),
            device,
            options,
        })
    }

    /// Enumerates every mode and also reports what could not be mapped.
    pub fn probe(&self) -> Result<CameraProbe, CaptureError> {
        let formats = self
            .device
            .enum_formats()
            .map_err(|error| map_io_error(&self.path, &error))?;
        let probes: Vec<FormatProbe> = formats
            .iter()
            .map(|format| self.probe_format(format.fourcc))
            .collect();
        Ok(map_probe(&probes))
    }

    fn probe_format(&self, fourcc: FourCC) -> FormatProbe {
        let sizes = match self.device.enum_framesizes(fourcc) {
            Err(error) => SizesProbe::Failed(error.to_string()),
            Ok(entries) => {
                let mut discrete = Vec::new();
                let mut stepwise = false;
                for entry in entries {
                    match entry.size {
                        FrameSizeEnum::Discrete(size) => {
                            discrete.push(self.probe_size(fourcc, size.width, size.height));
                        }
                        FrameSizeEnum::Stepwise(_) => stepwise = true,
                    }
                }
                if stepwise {
                    SizesProbe::Stepwise
                } else {
                    SizesProbe::Discrete(discrete)
                }
            }
        };
        FormatProbe {
            fourcc: fourcc.to_string(),
            sizes,
        }
    }

    fn probe_size(&self, fourcc: FourCC, width: u32, height: u32) -> SizeProbe {
        let intervals = match self.device.enum_frameintervals(fourcc, width, height) {
            Err(error) => IntervalProbe::Failed(error.to_string()),
            Ok(entries)
                if entries
                    .iter()
                    .any(|entry| matches!(entry.interval, FrameIntervalEnum::Stepwise(_))) =>
            {
                IntervalProbe::Stepwise
            }
            Ok(entries) => IntervalProbe::Discrete(
                entries
                    .iter()
                    .filter_map(|entry| match &entry.interval {
                        FrameIntervalEnum::Discrete(fraction) => {
                            Some((fraction.numerator, fraction.denominator))
                        }
                        FrameIntervalEnum::Stepwise(_) => None,
                    })
                    .collect(),
            ),
        };
        SizeProbe {
            width,
            height,
            intervals,
        }
    }

    fn fail(&self, error: &io::Error) -> CaptureError {
        map_io_error(&self.path, error)
    }
}

impl CameraPort for V4lCamera {
    fn capabilities(&self) -> Result<CameraCapabilities, CaptureError> {
        Ok(self.probe()?.capabilities)
    }

    fn open(&self, request: &CaptureMode) -> Result<Box<dyn FrameSource + '_>, CaptureError> {
        let wanted = Format::new(
            request.size.width,
            request.size.height,
            FourCC::new(&request.format.bytes()),
        );
        let applied = self
            .device
            .set_format(&wanted)
            .map_err(|error| self.fail(&error))?;
        let params = CaptureParameters::new(Fraction::new(
            request.interval.numerator(),
            request.interval.denominator(),
        ));
        let params = self
            .device
            .set_params(&params)
            .map_err(|error| self.fail(&error))?;

        let negotiated = CaptureMode {
            format: PixelFormat::parse(&applied.fourcc.to_string())?,
            size: FrameSize::new(applied.width, applied.height)?,
            interval: FrameInterval::new(params.interval.numerator, params.interval.denominator)?,
        };
        let stream =
            Stream::with_buffers(&self.device, Type::VideoCapture, self.options.buffer_count)
                .map_err(|error| self.fail(&error))?;
        Ok(Box::new(V4lSource {
            handle: stream.handle(),
            stream,
            // An H264 stream cannot be decoded before its keyframe, and some
            // cameras send it only as the stream starts.
            gate: (negotiated.format.bytes() == H264)
                .then(|| KeyframeGate::new(self.keyframe_restarts)),
            started: false,
            first_frame_timeout: self.options.first_frame_timeout,
            poll_timeout_ms: i32::try_from(self.options.poll_timeout.as_millis())
                .unwrap_or(i32::MAX),
            path: self.path.clone(),
            negotiated,
            validate_jpeg: negotiated.format.bytes() == MJPEG,
            idle_polls: 0,
            stall_limit: self.options.stall_limit,
        }))
    }
}

/// `POLLIN` on Linux: a filled buffer can be dequeued.
const POLLIN: i16 = 0x0001;

struct V4lSource<'a> {
    stream: Stream<'a>,
    /// Holds back an H264 stream's frames until its first keyframe.
    gate: Option<KeyframeGate>,
    handle: Arc<Handle>,
    /// False until the first `next` has started the stream and returned a frame.
    started: bool,
    first_frame_timeout: Duration,
    poll_timeout_ms: i32,
    path: String,
    negotiated: CaptureMode,
    validate_jpeg: bool,
    idle_polls: u32,
    stall_limit: u32,
}

impl V4lSource<'_> {
    fn idle(&mut self) -> Result<Option<RawFrame>, CaptureError> {
        self.idle_polls += 1;
        if self.idle_polls >= self.stall_limit {
            return Err(CaptureError::Acquisition(format!(
                "{}: no frame for {} consecutive polls",
                self.path, self.idle_polls
            )));
        }
        Ok(None)
    }
}

impl FrameSource for V4lSource<'_> {
    fn negotiated(&self) -> CaptureMode {
        self.negotiated
    }

    fn next_frame(&mut self) -> Result<Option<RawFrame>, CaptureError> {
        // `Stream::next` must never time out after the stream is running: it
        // would re-queue a buffer the driver already owns (EINVAL). So wait for
        // a filled buffer with `poll` first and only call `next` when one is ready.
        if self.started {
            match self.handle.poll(POLLIN, self.poll_timeout_ms) {
                Ok(0) => return self.idle(),
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return self.idle(),
                Err(error) => return Err(map_io_error(&self.path, &error)),
            }
        } else {
            self.stream.set_timeout(self.first_frame_timeout);
        }
        // Copy the frame out first: the borrowed buffer must end before the stream is touched again.
        let received = self.stream.next().map(|(buffer, meta)| {
            frame_from_buffer(
                buffer,
                meta.bytesused,
                meta.timestamp.sec,
                meta.timestamp.usec,
            )
        });
        if !self.started {
            self.stream.clear_timeout();
            self.started = true;
        }
        let mut frame = match received {
            Ok(frame) => frame?,
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {
                return Err(CaptureError::Acquisition(format!(
                    "{}: the camera delivered no frame in {} ms after starting",
                    self.path,
                    self.first_frame_timeout.as_millis()
                )));
            }
            Err(error) => return Err(map_io_error(&self.path, &error)),
        };
        self.idle_polls = 0;
        if let Some(gate) = self.gate.as_mut() {
            match gate.judge(has_keyframe(&frame.data)) {
                Verdict::Deliver => {}
                Verdict::Hold => return Ok(None),
                // Ends the session; `run` opens the camera again.
                Verdict::Restart => {
                    return Err(CaptureError::Acquisition(format!(
                        "{}: {KEYFRAME_MISSING}",
                        self.path
                    )))
                }
            }
        }
        if self.validate_jpeg {
            let valid_len = validate_mjpeg(&frame.data, self.negotiated.size)?;
            frame.data.truncate(valid_len);
        }
        Ok(Some(frame))
    }
}

/// The platform catalog: real `/dev/video*` nodes.
#[derive(Debug, Default, Clone, Copy)]
pub struct V4lCatalog {
    pub options: V4lOptions,
}

impl CameraCatalog for V4lCatalog {
    fn list(&self) -> Vec<CameraInfo> {
        list_cameras()
    }

    fn probe(&self, device_path: &str) -> Result<CameraProbe, CaptureError> {
        V4lCamera::open_path(device_path, self.options)?.probe()
    }

    fn run(
        &self,
        device_path: &str,
        request: &CaptureMode,
        config: SessionConfig,
        cancel: &CancelFlag,
        observer: &mut dyn SessionObserver,
    ) -> Result<SessionOutcome, CaptureError> {
        let mut restarts_left = MAX_RESTARTS;
        loop {
            let mut camera = V4lCamera::open_path(device_path, self.options)?;
            camera.keyframe_restarts = restarts_left;
            let outcome = run_capture_observed(&camera, request, config, cancel, &mut *observer)?;
            let missing = matches!(
                &outcome.end,
                EndReason::Failed { error } if error.message.contains(KEYFRAME_MISSING)
            );
            if !missing || restarts_left == 0 || cancel.is_cancelled() {
                return Ok(outcome);
            }
            restarts_left -= 1;
            // Close the camera completely and give it a moment before opening it again.
            drop(camera);
            std::thread::sleep(REOPEN_PAUSE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loopback_driver_is_recognised_by_name() {
        assert!(is_loopback_driver("v4l2 loopback"));
        assert!(is_loopback_driver("V4L2 Loopback"));
        assert!(!is_loopback_driver("uvcvideo"));
        assert!(!is_loopback_driver(""));
    }
}
