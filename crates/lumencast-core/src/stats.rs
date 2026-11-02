//! Frame and error counters. FPS is measured from capture timestamps, never
//! inferred from the requested interval or from preview scheduling.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    Acquisition,
    Decode,
}

#[derive(Debug, Clone, Default)]
pub struct CaptureStats {
    frames_captured: u64,
    frames_previewed: u64,
    acquisition_errors: u64,
    decode_errors: u64,
    out_of_order_timestamps: u64,
    first_timestamp_us: Option<u64>,
    last_timestamp_us: Option<u64>,
    timed_frames: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
    pub frames_captured: u64,
    pub frames_previewed: u64,
    pub acquisition_errors: u64,
    pub decode_errors: u64,
    pub out_of_order_timestamps: u64,
    /// `None` until at least two ordered capture timestamps were observed.
    pub measured_fps: Option<f64>,
}

impl CaptureStats {
    /// A frame was captured and decoded; `timestamp_us` is the raw capture time.
    pub fn record_frame(&mut self, timestamp_us: u64) {
        self.frames_captured += 1;
        if self
            .last_timestamp_us
            .is_some_and(|last| timestamp_us < last)
        {
            self.out_of_order_timestamps += 1;
            return;
        }
        self.first_timestamp_us.get_or_insert(timestamp_us);
        self.last_timestamp_us = Some(timestamp_us);
        self.timed_frames += 1;
    }

    /// A frame was handed to the preview. Independent from capture timing.
    pub fn record_preview(&mut self) {
        self.frames_previewed += 1;
    }

    pub fn record_error(&mut self, kind: ErrorKind) {
        match kind {
            ErrorKind::Acquisition => self.acquisition_errors += 1,
            ErrorKind::Decode => self.decode_errors += 1,
        }
    }

    pub fn snapshot(&self) -> StatsSnapshot {
        StatsSnapshot {
            frames_captured: self.frames_captured,
            frames_previewed: self.frames_previewed,
            acquisition_errors: self.acquisition_errors,
            decode_errors: self.decode_errors,
            out_of_order_timestamps: self.out_of_order_timestamps,
            measured_fps: self.measured_fps(),
        }
    }

    fn measured_fps(&self) -> Option<f64> {
        let (first, last) = (self.first_timestamp_us?, self.last_timestamp_us?);
        let span_us = last.checked_sub(first).filter(|span| *span > 0)?;
        if self.timed_frames < 2 {
            return None;
        }
        Some((self.timed_frames - 1) as f64 * 1_000_000.0 / span_us as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fps_is_measured_from_capture_timestamps() {
        let mut stats = CaptureStats::default();
        for index in 0..31u64 {
            stats.record_frame(index * 33_333);
        }
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.frames_captured, 31);
        let fps = snapshot.measured_fps.unwrap();
        assert!((fps - 30.0).abs() < 0.01, "measured {fps}");
    }

    #[test]
    fn fps_is_unknown_with_fewer_than_two_timestamps() {
        let mut stats = CaptureStats::default();
        assert_eq!(stats.snapshot().measured_fps, None);
        stats.record_frame(1_000);
        assert_eq!(stats.snapshot().measured_fps, None);
    }

    #[test]
    fn errors_are_counted_by_kind_and_are_not_frames() {
        let mut stats = CaptureStats::default();
        stats.record_error(ErrorKind::Acquisition);
        stats.record_error(ErrorKind::Decode);
        stats.record_error(ErrorKind::Decode);
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.acquisition_errors, 1);
        assert_eq!(snapshot.decode_errors, 2);
        assert_eq!(snapshot.frames_captured, 0);
    }

    #[test]
    fn preview_scheduling_does_not_change_measured_fps() {
        let mut stats = CaptureStats::default();
        stats.record_frame(0);
        stats.record_frame(100_000);
        for _ in 0..50 {
            stats.record_preview();
        }
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.frames_previewed, 50);
        assert!((snapshot.measured_fps.unwrap() - 10.0).abs() < 0.01);
    }

    #[test]
    fn backwards_timestamps_are_counted_and_excluded_from_fps() {
        let mut stats = CaptureStats::default();
        stats.record_frame(100_000);
        stats.record_frame(50_000);
        stats.record_frame(200_000);
        let snapshot = stats.snapshot();
        assert_eq!(snapshot.frames_captured, 3);
        assert_eq!(snapshot.out_of_order_timestamps, 1);
        assert!((snapshot.measured_fps.unwrap() - 10.0).abs() < 0.01);
    }

    #[test]
    fn snapshot_serializes_in_camel_case() {
        let json = serde_json::to_value(CaptureStats::default().snapshot()).unwrap();
        assert_eq!(json["framesCaptured"], 0);
        assert!(json["measuredFps"].is_null());
    }
}
