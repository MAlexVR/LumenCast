//! Events pushed to the UI while a capture runs.

use crate::audio::{AudioPath, PathStatus};
use crate::output::VirtualCameraStatus;
use crate::preview::{AutoFramingStatus, BackgroundStatus};
use lumencast_core::error::ErrorPayload;
use lumencast_core::mode::NegotiationReport;
use lumencast_core::session::SessionOutcome;
use lumencast_core::stats::StatsSnapshot;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AppEvent {
    /// Requested versus negotiated mode, sent once before the first frame.
    Negotiated {
        report: NegotiationReport,
    },
    /// A frame chosen for the preview (throttled by capture time).
    Frame {
        timestamp_us: u64,
        jpeg_base64: String,
    },
    Stats {
        stats: StatsSnapshot,
    },
    /// How the auto framing is doing; sent when it changes noticeably.
    AutoFraming {
        status: AutoFramingStatus,
    },
    /// How the two audio paths are doing; sent when either changes.
    AudioStatus {
        microphone: PathStatus,
        speakers: PathStatus,
    },
    /// The level going into and out of an audio path, about ten times a second.
    AudioLevels {
        path: AudioPath,
        input_db: f32,
        output_db: f32,
    },
    /// How the virtual camera is doing; sent when it changes.
    VirtualCamera {
        status: VirtualCameraStatus,
    },
    /// How the background effect is doing; sent when it changes.
    Background {
        status: BackgroundStatus,
    },
    /// A frame chosen for the preview could not be decoded and was dropped;
    /// `total` counts them for the whole capture. The capture keeps running.
    PreviewDropped {
        total: u64,
    },
    /// Exactly one of `outcome` and `error` is set.
    Ended {
        outcome: Option<SessionOutcome>,
        error: Option<ErrorPayload>,
    },
}

pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, event: AppEvent);
}
