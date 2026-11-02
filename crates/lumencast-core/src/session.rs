//! Capture use-case: validate the requested mode, open the camera, report what
//! was negotiated and pull frames until cancelled, limited or failed. No
//! effects and no models are involved.

use crate::cancel::CancelFlag;
use crate::error::{CaptureError, ErrorPayload};
use crate::mode::{CameraCapabilities, CaptureMode, NegotiationReport};
use crate::stats::{CaptureStats, ErrorKind, StatsSnapshot};
use serde::Serialize;

/// One encoded frame as delivered by the camera, with its raw capture time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawFrame {
    pub timestamp_us: u64,
    pub data: Vec<u8>,
}

/// A camera that can list its capabilities and be opened with a mode.
pub trait CameraPort {
    fn capabilities(&self) -> Result<CameraCapabilities, CaptureError>;
    /// The source may borrow the camera (the V4L2 stream borrows its device),
    /// so it never outlives it and need not be `Send`: the whole session runs
    /// on one blocking thread.
    fn open(&self, request: &CaptureMode) -> Result<Box<dyn FrameSource + '_>, CaptureError>;
}

/// An open camera stream. `next_frame` must return within a bounded time so
/// cancellation stays responsive.
pub trait FrameSource {
    /// The mode the driver actually granted, read back after opening.
    fn negotiated(&self) -> CaptureMode;
    /// `Ok(None)` means no frame arrived within the source's short poll
    /// timeout: it is not an error and lets the caller observe cancellation.
    /// `Decode` errors are skipped and counted; any other error ends the session.
    fn next_frame(&mut self) -> Result<Option<RawFrame>, CaptureError>;
}

/// Receives what the session learns while it runs.
pub trait SessionObserver {
    /// Called once, after the camera is open and before the first frame.
    fn negotiated(&mut self, _report: &NegotiationReport) {}

    /// Returns whether the frame was handed to the preview. `stats` already
    /// counts this frame as captured but not yet as previewed.
    fn frame(&mut self, frame: &RawFrame, stats: &StatsSnapshot) -> bool;
}

impl<T: SessionObserver + ?Sized> SessionObserver for &mut T {
    fn negotiated(&mut self, report: &NegotiationReport) {
        (**self).negotiated(report);
    }

    fn frame(&mut self, frame: &RawFrame, stats: &StatsSnapshot) -> bool {
        (**self).frame(frame, stats)
    }
}

/// Adapts a plain frame callback for callers that need nothing else.
struct FrameCallback<F>(F);

impl<F: FnMut(&RawFrame) -> bool> SessionObserver for FrameCallback<F> {
    fn frame(&mut self, frame: &RawFrame, _stats: &StatsSnapshot) -> bool {
        (self.0)(frame)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionConfig {
    pub max_frames: Option<u64>,
    pub max_consecutive_decode_errors: u32,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_frames: None,
            max_consecutive_decode_errors: 30,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EndReason {
    Cancelled,
    FrameLimit,
    Failed { error: ErrorPayload },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOutcome {
    pub report: NegotiationReport,
    pub stats: StatsSnapshot,
    pub end: EndReason,
}

/// Failures before the stream starts are `Err`; failures after it starts end
/// the session with `EndReason::Failed` so the counters are never lost.
/// The observer is told the negotiated mode and receives every frame.
pub fn run_capture(
    port: &dyn CameraPort,
    request: &CaptureMode,
    config: SessionConfig,
    cancel: &CancelFlag,
    on_frame: impl FnMut(&RawFrame) -> bool,
) -> Result<SessionOutcome, CaptureError> {
    run_capture_observed(port, request, config, cancel, FrameCallback(on_frame))
}

pub fn run_capture_observed(
    port: &dyn CameraPort,
    request: &CaptureMode,
    config: SessionConfig,
    cancel: &CancelFlag,
    mut observer: impl SessionObserver,
) -> Result<SessionOutcome, CaptureError> {
    let requested = port.capabilities()?.validate(request)?;
    let mut source = port.open(&requested)?;
    let report = NegotiationReport::new(requested, source.negotiated());
    observer.negotiated(&report);
    let mut stats = CaptureStats::default();
    let mut consecutive_decode_errors = 0;

    let end = loop {
        if cancel.is_cancelled() {
            break EndReason::Cancelled;
        }
        if config
            .max_frames
            .is_some_and(|limit| stats.snapshot().frames_captured >= limit)
        {
            break EndReason::FrameLimit;
        }
        match source.next_frame() {
            Ok(None) => {}
            Ok(Some(frame)) => {
                consecutive_decode_errors = 0;
                stats.record_frame(frame.timestamp_us);
                if observer.frame(&frame, &stats.snapshot()) {
                    stats.record_preview();
                }
            }
            Err(CaptureError::Cancelled) => break EndReason::Cancelled,
            Err(error @ CaptureError::Decode(_)) => {
                stats.record_error(ErrorKind::Decode);
                consecutive_decode_errors += 1;
                if consecutive_decode_errors >= config.max_consecutive_decode_errors {
                    break EndReason::Failed {
                        error: error.payload(),
                    };
                }
            }
            Err(error) => {
                if matches!(error, CaptureError::Acquisition(_)) {
                    stats.record_error(ErrorKind::Acquisition);
                }
                break EndReason::Failed {
                    error: error.payload(),
                };
            }
        }
    };

    Ok(SessionOutcome {
        report,
        stats: stats.snapshot(),
        end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mode::{FrameInterval, FrameSize, PixelFormat};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    fn mode(fourcc: &str, w: u32, h: u32, den: u32) -> CaptureMode {
        CaptureMode {
            format: PixelFormat::parse(fourcc).unwrap(),
            size: FrameSize::new(w, h).unwrap(),
            interval: FrameInterval::new(1, den).unwrap(),
        }
    }

    fn frame(timestamp_us: u64) -> RawFrame {
        RawFrame {
            timestamp_us,
            data: vec![0xFF, 0xD8],
        }
    }

    struct Scripted {
        negotiated: CaptureMode,
        script: VecDeque<Result<Option<RawFrame>, CaptureError>>,
    }

    impl FrameSource for Scripted {
        fn negotiated(&self) -> CaptureMode {
            self.negotiated
        }
        fn next_frame(&mut self) -> Result<Option<RawFrame>, CaptureError> {
            self.script
                .pop_front()
                .unwrap_or_else(|| Err(CaptureError::Acquisition("script exhausted".into())))
        }
    }

    struct FakeCamera {
        capabilities: CameraCapabilities,
        source: Mutex<Option<Box<dyn FrameSource>>>,
        opened: Mutex<bool>,
    }

    impl FakeCamera {
        fn new(
            negotiated: CaptureMode,
            script: Vec<Result<Option<RawFrame>, CaptureError>>,
        ) -> Self {
            Self::with_source(Box::new(Scripted {
                negotiated,
                script: script.into(),
            }))
        }

        fn with_source(source: Box<dyn FrameSource>) -> Self {
            Self {
                capabilities: CameraCapabilities::new([
                    mode("MJPG", 1280, 720, 30),
                    mode("MJPG", 1280, 720, 15),
                ]),
                source: Mutex::new(Some(source)),
                opened: Mutex::new(false),
            }
        }
        fn was_opened(&self) -> bool {
            *self.opened.lock().unwrap()
        }
    }

    impl CameraPort for FakeCamera {
        fn capabilities(&self) -> Result<CameraCapabilities, CaptureError> {
            Ok(self.capabilities.clone())
        }
        fn open(&self, _request: &CaptureMode) -> Result<Box<dyn FrameSource + '_>, CaptureError> {
            *self.opened.lock().unwrap() = true;
            Ok(self.source.lock().unwrap().take().unwrap())
        }
    }

    fn ok_frames(count: u64) -> Vec<Result<Option<RawFrame>, CaptureError>> {
        (0..count)
            .map(|index| Ok(Some(frame(index * 33_333))))
            .collect()
    }

    fn limited(frames: u64) -> SessionConfig {
        SessionConfig {
            max_frames: Some(frames),
            ..SessionConfig::default()
        }
    }

    #[test]
    fn unsupported_request_fails_before_the_camera_is_opened() {
        let camera = FakeCamera::new(mode("MJPG", 1280, 720, 30), ok_frames(5));
        let result = run_capture(
            &camera,
            &mode("MJPG", 1280, 720, 60),
            limited(5),
            &CancelFlag::new(),
            |_| true,
        );
        assert_eq!(result.unwrap_err().code(), "mode_unsupported");
        assert!(!camera.was_opened());
    }

    #[test]
    fn exact_negotiation_streams_until_the_frame_limit() {
        let wanted = mode("MJPG", 1280, 720, 30);
        let camera = FakeCamera::new(wanted, ok_frames(10));
        let outcome =
            run_capture(&camera, &wanted, limited(5), &CancelFlag::new(), |_| true).unwrap();
        assert!(outcome.report.is_exact());
        assert_eq!(outcome.end, EndReason::FrameLimit);
        assert_eq!(outcome.stats.frames_captured, 5);
        assert_eq!(outcome.stats.frames_previewed, 5);
        assert!((outcome.stats.measured_fps.unwrap() - 30.0).abs() < 0.01);
    }

    #[test]
    fn a_driver_adjusted_mode_is_reported_not_hidden() {
        let requested = mode("MJPG", 1280, 720, 30);
        let granted = mode("MJPG", 1280, 720, 15);
        let camera = FakeCamera::new(granted, ok_frames(3));
        let outcome = run_capture(&camera, &requested, limited(3), &CancelFlag::new(), |_| {
            true
        })
        .unwrap();
        assert!(!outcome.report.is_exact());
        assert_eq!(outcome.report.requested, requested);
        assert_eq!(outcome.report.negotiated, granted);
    }

    #[test]
    fn decode_errors_are_counted_and_skipped() {
        let wanted = mode("MJPG", 1280, 720, 30);
        let script = vec![
            Err(CaptureError::Decode("first packet invalid".into())),
            Ok(Some(frame(0))),
            Ok(Some(frame(33_333))),
        ];
        let camera = FakeCamera::new(wanted, script);
        let outcome =
            run_capture(&camera, &wanted, limited(2), &CancelFlag::new(), |_| true).unwrap();
        assert_eq!(outcome.stats.decode_errors, 1);
        assert_eq!(outcome.stats.frames_captured, 2);
        assert_eq!(outcome.end, EndReason::FrameLimit);
    }

    #[test]
    fn too_many_consecutive_decode_errors_fail_the_session() {
        let wanted = mode("MJPG", 1280, 720, 30);
        let script = (0..10)
            .map(|_| Err(CaptureError::Decode("bad".into())))
            .collect();
        let camera = FakeCamera::new(wanted, script);
        let config = SessionConfig {
            max_frames: None,
            max_consecutive_decode_errors: 3,
        };
        let outcome = run_capture(&camera, &wanted, config, &CancelFlag::new(), |_| true).unwrap();
        match outcome.end {
            EndReason::Failed { error } => assert_eq!(error.code, "decode_failed"),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(outcome.stats.decode_errors, 3);
    }

    #[test]
    fn acquisition_errors_end_the_session_and_keep_the_counters() {
        let wanted = mode("MJPG", 1280, 720, 30);
        let script = vec![
            Ok(Some(frame(0))),
            Ok(Some(frame(33_333))),
            Err(CaptureError::Acquisition("usb disconnect".into())),
        ];
        let camera = FakeCamera::new(wanted, script);
        let outcome = run_capture(
            &camera,
            &wanted,
            SessionConfig::default(),
            &CancelFlag::new(),
            |_| true,
        )
        .unwrap();
        match outcome.end {
            EndReason::Failed { error } => assert_eq!(error.code, "acquisition_failed"),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(outcome.stats.frames_captured, 2);
        assert_eq!(outcome.stats.acquisition_errors, 1);
    }

    #[test]
    fn cancellation_stops_the_stream() {
        let wanted = mode("MJPG", 1280, 720, 30);
        let camera = FakeCamera::new(wanted, ok_frames(20));
        let cancel = CancelFlag::new();
        let handle = cancel.clone();
        let mut seen = 0;
        let outcome = run_capture(&camera, &wanted, SessionConfig::default(), &cancel, |_| {
            seen += 1;
            if seen == 3 {
                handle.cancel();
            }
            true
        })
        .unwrap();
        assert_eq!(outcome.end, EndReason::Cancelled);
        assert_eq!(outcome.stats.frames_captured, 3);
    }

    #[test]
    fn preview_count_only_includes_frames_the_sink_accepted() {
        let wanted = mode("MJPG", 1280, 720, 30);
        let camera = FakeCamera::new(wanted, ok_frames(6));
        let mut index = 0;
        let outcome = run_capture(&camera, &wanted, limited(6), &CancelFlag::new(), |_| {
            index += 1;
            index % 2 == 0
        })
        .unwrap();
        assert_eq!(outcome.stats.frames_captured, 6);
        assert_eq!(outcome.stats.frames_previewed, 3);
    }

    #[test]
    fn observer_hears_the_negotiated_mode_before_the_first_frame() {
        use std::cell::RefCell;
        use std::rc::Rc;
        #[derive(Default)]
        struct Recorder {
            events: Rc<RefCell<Vec<String>>>,
            last_stats: Rc<RefCell<Option<StatsSnapshot>>>,
        }
        impl SessionObserver for Recorder {
            fn negotiated(&mut self, report: &NegotiationReport) {
                self.events
                    .borrow_mut()
                    .push(format!("negotiated exact={}", report.is_exact()));
            }
            fn frame(&mut self, _frame: &RawFrame, stats: &StatsSnapshot) -> bool {
                self.events
                    .borrow_mut()
                    .push(format!("frame {}", stats.frames_captured));
                *self.last_stats.borrow_mut() = Some(stats.clone());
                true
            }
        }
        let requested = mode("MJPG", 1280, 720, 30);
        let granted = mode("MJPG", 1280, 720, 15);
        let camera = FakeCamera::new(granted, ok_frames(2));
        let recorder = Recorder::default();
        let (events, last_stats) = (recorder.events.clone(), recorder.last_stats.clone());
        run_capture_observed(
            &camera,
            &requested,
            limited(2),
            &CancelFlag::new(),
            recorder,
        )
        .unwrap();
        assert_eq!(
            *events.borrow(),
            ["negotiated exact=false", "frame 1", "frame 2"]
        );
        // Stats include the frame as captured, but not yet as previewed.
        assert_eq!(last_stats.borrow().as_ref().unwrap().frames_previewed, 1);
    }

    #[test]
    fn idle_polls_are_not_frames_or_errors() {
        let wanted = mode("MJPG", 1280, 720, 30);
        let script = vec![
            Ok(None),
            Ok(None),
            Ok(Some(frame(0))),
            Ok(None),
            Ok(Some(frame(33_333))),
        ];
        let camera = FakeCamera::new(wanted, script);
        let outcome =
            run_capture(&camera, &wanted, limited(2), &CancelFlag::new(), |_| true).unwrap();
        assert_eq!(outcome.stats.frames_captured, 2);
        assert_eq!(outcome.stats.decode_errors, 0);
        assert_eq!(outcome.stats.acquisition_errors, 0);
        assert_eq!(outcome.end, EndReason::FrameLimit);
    }

    #[test]
    fn cancellation_is_observed_while_the_camera_is_idle() {
        struct IdleThenCancel {
            negotiated: CaptureMode,
            cancel: CancelFlag,
            polls: u32,
        }
        impl FrameSource for IdleThenCancel {
            fn negotiated(&self) -> CaptureMode {
                self.negotiated
            }
            fn next_frame(&mut self) -> Result<Option<RawFrame>, CaptureError> {
                self.polls += 1;
                if self.polls == 3 {
                    self.cancel.cancel();
                }
                Ok(None)
            }
        }
        let wanted = mode("MJPG", 1280, 720, 30);
        let cancel = CancelFlag::new();
        let camera = FakeCamera::with_source(Box::new(IdleThenCancel {
            negotiated: wanted,
            cancel: cancel.clone(),
            polls: 0,
        }));
        let outcome = run_capture(&camera, &wanted, SessionConfig::default(), &cancel, |_| {
            true
        })
        .unwrap();
        assert_eq!(outcome.end, EndReason::Cancelled);
        assert_eq!(outcome.stats.frames_captured, 0);
    }

    #[test]
    fn outcome_serializes_with_a_tagged_end_reason() {
        let wanted = mode("MJPG", 1280, 720, 30);
        let camera = FakeCamera::new(wanted, ok_frames(1));
        let outcome =
            run_capture(&camera, &wanted, limited(1), &CancelFlag::new(), |_| true).unwrap();
        let json = serde_json::to_value(&outcome).unwrap();
        assert_eq!(json["end"]["kind"], "frameLimit");
        assert_eq!(json["stats"]["framesCaptured"], 1);
        assert_eq!(json["report"]["differences"], serde_json::json!([]));
    }
}
