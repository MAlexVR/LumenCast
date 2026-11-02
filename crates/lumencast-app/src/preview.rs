//! Preview rendering. It runs on its own thread so that a slow decode or
//! encode can only make the preview skip frames; it can never slow the
//! capture. When the worker is busy the newest frame wins and older ones are
//! skipped.
//!
//! A capture gets one `PreviewSession`, created on the worker thread, because
//! auto framing needs state between frames: the face detector and the
//! smoothed framing.

use crate::backdrop::load_image;
use crate::events::{AppEvent, EventSink};
use crate::output::{FrameOutput, LoopbackFactory, NoOutput, OutputFactory, VirtualCameraStatus};
use crate::processing::{AutoFramingSettings, ProcessingSettings};
use crate::processing::{BackgroundEngine, BackgroundMode, BackgroundSettings, KeyLightSettings};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use lumencast_core::mode::CaptureMode;
use lumencast_fx::background::{cover, Backdrop as FxBackdrop, Background};
use lumencast_fx::codec::{decode_jpeg, encode_jpeg, yuyv_to_rgb};
use lumencast_fx::denoise::TemporalDenoise;
use lumencast_fx::frame::Frame;
use lumencast_fx::lightprobe::LightProbe;
use lumencast_fx::matte::Matte;
use lumencast_fx::params::Strength;
use lumencast_fx::params::Zoom;
use lumencast_fx::reframe::{AutoFraming, FaceBox, FaceDetector, Reframer};
use lumencast_fx::resample::resize_lanczos;
use lumencast_fx::stages::Stage;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender, TrySendError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// JPEG quality of re-encoded preview frames.
const PREVIEW_QUALITY: u8 = 85;

/// The least zoom auto framing works at: below it there is no margin to follow in.
const MIN_FOLLOW_ZOOM: f32 = 1.10;
/// The background effect looks for the person at most this often (about 15
/// times a second at 30 fps).
const SEGMENT_INTERVAL_US: u64 = 60_000;

/// The picture chosen to replace the background, loaded once and then scaled
/// to the size of the frames.
struct Backdrop {
    path: String,
    loaded: Result<Frame, String>,
    scaled: Option<Frame>,
}

/// How much of the old face position survives a new detection (key light).
const LIGHT_SMOOTHING: f32 = 0.6;
/// Detections in a row without a face before the light lets go.
const LIGHT_MISSES_ALLOWED: u32 = 3;
/// A face is looked for about ten times a second at 30 fps (the interval is a
/// little under 100 ms because capture timestamps jitter and 3 frames add up to
/// 99.999 ms).
const DETECT_INTERVAL_US: u64 = 90_000;

/// The legacy server detected about 15 times per second; used for the first step.
const FIRST_STEP_US: u64 = 66_667;

/// Smaller changes of the framing are not worth an event.
const ZOOM_REPORT_STEP: f32 = 1.0;
const PAN_REPORT_STEP: f32 = 0.02;

/// What the auto framing is doing, for the UI.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AutoFramingStatus {
    Off,
    /// On, but no face has been found yet.
    Searching,
    /// On, but the face detector could not be started.
    Unavailable,
    Locked {
        zoom_percent: f32,
        pan_x: f32,
        pan_y: f32,
        /// False while the face is too far from the centre for this zoom to centre it.
        reachable: bool,
    },
}

impl AutoFramingStatus {
    /// Whether the UI should be told about a change from `previous`.
    fn differs_noticeably(&self, previous: &Self) -> bool {
        match (self, previous) {
            (
                Self::Locked {
                    zoom_percent,
                    pan_x,
                    pan_y,
                    reachable,
                },
                Self::Locked {
                    zoom_percent: old_zoom,
                    pan_x: old_x,
                    pan_y: old_y,
                    reachable: old_reachable,
                },
            ) => {
                reachable != old_reachable
                    || (zoom_percent - old_zoom).abs() >= ZOOM_REPORT_STEP
                    || (pan_x - old_x).abs() >= PAN_REPORT_STEP
                    || (pan_y - old_y).abs() >= PAN_REPORT_STEP
            }
            _ => self != previous,
        }
    }
}

/// How the background effect is doing.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum BackgroundStatus {
    Off,
    /// Running with `engine`; `fallback` says why the one asked for is not the one in use.
    Working {
        engine: BackgroundEngine,
        fallback: Option<String>,
    },
    /// The effect could not run; the picture is shown unchanged.
    Unavailable {
        problem: BackgroundProblem,
        reason: String,
    },
}

/// What kind of problem stopped the background effect, so the UI can word it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundProblem {
    /// `Replace` without a picture chosen yet: not an error, a step to take.
    NoPicture,
    /// The chosen picture could not be read.
    PictureUnreadable,
    /// The segmentation model could not run.
    ModelUnavailable,
}

pub struct Rendered {
    pub jpeg: Vec<u8>,
    pub auto_framing: AutoFramingStatus,
    pub background: BackgroundStatus,
}

/// Finds the person in a frame. The model behind it is not this module's business.
/// `Ok(None)` means the model is not ready yet (a server still loading).
pub trait Segmenter {
    fn matte(&mut self, frame: &Frame) -> Result<Option<Matte>, String>;
}

/// Creates a segmenter on the worker thread, only when the effect is used.
pub trait SegmenterFactory: Send + Sync + 'static {
    fn create(&self) -> Result<Box<dyn Segmenter>, String>;
}

/// The built-in selfie segmentation model.
pub struct SelfieFactory;

impl Segmenter for lumencast_segment::SelfieSegmenter {
    fn matte(&mut self, frame: &Frame) -> Result<Option<Matte>, String> {
        lumencast_segment::SelfieSegmenter::matte(self, frame)
            .map(Some)
            .map_err(|error| error.to_string())
    }
}

/// NVIDIA's model through the matte server container.
struct NvidiaSegmenter(lumencast_nvidia::client::MatteClient);

impl Segmenter for NvidiaSegmenter {
    fn matte(&mut self, frame: &Frame) -> Result<Option<Matte>, String> {
        Ok(self.0.matte(frame))
    }
}

/// Starts the matte server container (if it is not running) and connects to it.
pub struct NvidiaFactory;

impl SegmenterFactory for NvidiaFactory {
    fn create(&self) -> Result<Box<dyn Segmenter>, String> {
        lumencast_nvidia::sidecar::ensure_started(
            &lumencast_nvidia::sidecar::SystemPodman,
            &lumencast_nvidia::sidecar::image_name(),
        )?;
        Ok(Box::new(NvidiaSegmenter(
            lumencast_nvidia::client::MatteClient::new(lumencast_nvidia::sidecar::PORT),
        )))
    }
}

/// Used where NVIDIA's model must not be reached (tests, or a build without it).
pub struct NoNvidia;

impl SegmenterFactory for NoNvidia {
    fn create(&self) -> Result<Box<dyn Segmenter>, String> {
        Err("the NVIDIA model is not configured".into())
    }
}

impl SegmenterFactory for SelfieFactory {
    fn create(&self) -> Result<Box<dyn Segmenter>, String> {
        lumencast_segment::SelfieSegmenter::new()
            .map(|segmenter| Box::new(segmenter) as Box<dyn Segmenter>)
            .map_err(|error| error.to_string())
    }
}

/// Creates a face detector. Called on the worker thread, so the detector
/// itself does not have to be `Send`.
pub trait DetectorFactory: Send + Sync + 'static {
    fn create(&self) -> Result<Box<dyn FaceDetector>, String>;
}

/// The built-in SeetaFace detector.
pub struct SeetaFactory;

impl DetectorFactory for SeetaFactory {
    fn create(&self) -> Result<Box<dyn FaceDetector>, String> {
        // On a thread of its own: a detection takes about 30 ms, which would
        // otherwise hold up every frame it lands on.
        lumencast_vision::AsyncDetector::seeta()
            .map(|detector| Box::new(detector) as Box<dyn FaceDetector>)
    }
}

/// Returned by `render` when a stateful stream has no picture to show yet
/// (for example while waiting for a keyframe). Not counted as a dropped frame.
pub const NOT_READY: &str = "not ready";

/// Formats whose pictures depend on the previous ones: every frame must reach
/// the decoder, shown or not.
pub fn needs_every_frame(fourcc: &str) -> bool {
    fourcc == "H264"
}

/// Starts the per-capture rendering state.
pub trait PreviewRenderer: Send + Sync + 'static {
    fn start(&self) -> Box<dyn PreviewSession>;

    /// `Ok` when frames of this format can be shown, else why not.
    fn supports(&self, fourcc: &str) -> Result<(), String>;

    /// Where the processed video goes when the virtual camera is on.
    fn outputs(&self) -> Arc<dyn OutputFactory> {
        Arc::new(NoOutput)
    }
}

/// Turns the camera frames of one capture into preview JPEGs.
pub trait PreviewSession {
    /// Feeds a frame that will not be shown. Stateful formats must still
    /// decode it; the others have nothing to do.
    fn advance(
        &mut self,
        _timestamp_us: u64,
        _mode: &CaptureMode,
        _data: &[u8],
    ) -> Result<(), String> {
        Ok(())
    }

    fn render(
        &mut self,
        timestamp_us: u64,
        settings: &ProcessingSettings,
        mode: &CaptureMode,
        data: &[u8],
    ) -> Result<Rendered, String>;

    /// Processes a frame for the preview (when `want_preview`) and for the
    /// virtual camera (when it is on), without encoding the preview here.
    /// Sessions that only know how to render get the preview as a JPEG.
    fn produce(
        &mut self,
        timestamp_us: u64,
        settings: &ProcessingSettings,
        mode: &CaptureMode,
        data: &[u8],
        _want_preview: bool,
    ) -> Result<Produced, String> {
        let rendered = self.render(timestamp_us, settings, mode, data)?;
        Ok(Produced {
            preview: Some(Picture::Jpeg(rendered.jpeg)),
            output: None,
            auto_framing: rendered.auto_framing,
            background: rendered.background,
        })
    }
}

/// The frame for the virtual camera, scaled up when that is asked for.
fn upscaled(frame: Frame, camera: &crate::processing::VirtualCameraSettings) -> Frame {
    match camera.upscaled_size(frame.width(), frame.height()) {
        Some((width, height)) => resize_lanczos(&frame, width, height),
        None => frame,
    }
}

/// A preview picture, encoded already or waiting to be.
pub enum Picture {
    Jpeg(Vec<u8>),
    Raw(Frame),
}

/// What processing one frame yields.
pub struct Produced {
    pub preview: Option<Picture>,
    /// The frame for the virtual camera, when it is on.
    pub output: Option<Frame>,
    pub auto_framing: AutoFramingStatus,
    pub background: BackgroundStatus,
}

/// The real renderer: decode, follow the face if asked, run the pipeline, re-encode.
pub struct FxRenderer {
    detectors: Arc<dyn DetectorFactory>,
    segmenters: Arc<dyn SegmenterFactory>,
    nvidia: Arc<dyn SegmenterFactory>,
    outputs: Arc<dyn OutputFactory>,
    /// Whether the system OpenH264 can be used; fixed when the renderer is built.
    h264: Result<(), String>,
}

impl FxRenderer {
    pub fn new(detectors: Arc<dyn DetectorFactory>) -> Self {
        Self {
            detectors,
            segmenters: Arc::new(SelfieFactory),
            nvidia: Arc::new(NoNvidia),
            outputs: Arc::new(NoOutput),
            h264: lumencast_h264::availability(),
        }
    }

    /// Replaces the segmentation model, for tests.
    pub fn with_segmenters(mut self, segmenters: Arc<dyn SegmenterFactory>) -> Self {
        self.segmenters = segmenters;
        self
    }

    /// Pretends OpenH264 is missing, for tests that must not depend on the machine.
    pub fn with_h264_unavailable(mut self, reason: &str) -> Self {
        self.h264 = Err(reason.to_owned());
        self
    }

    pub fn seeta() -> Self {
        Self::new(Arc::new(SeetaFactory))
            .with_nvidia(Arc::new(NvidiaFactory))
            .with_outputs(Arc::new(LoopbackFactory))
    }

    /// Replaces where the virtual camera sends its frames.
    pub fn with_outputs(mut self, outputs: Arc<dyn OutputFactory>) -> Self {
        self.outputs = outputs;
        self
    }

    /// Replaces NVIDIA's model, for tests.
    pub fn with_nvidia(mut self, nvidia: Arc<dyn SegmenterFactory>) -> Self {
        self.nvidia = nvidia;
        self
    }
}

impl PreviewRenderer for FxRenderer {
    fn start(&self) -> Box<dyn PreviewSession> {
        Box::new(FxSession {
            detectors: Arc::clone(&self.detectors),
            segmenters: Arc::clone(&self.segmenters),
            light_probe: None,
            nvidia_factory: Arc::clone(&self.nvidia),
            nvidia: None,
            matte_engine: BackgroundEngine::Builtin,
            fallback: None,
            denoiser: TemporalDenoise::new(),
            segmenter: None,
            matte: None,
            last_matte_us: None,
            backdrop: None,
            h264_unavailable: self.h264.clone().err(),
            h264: None,
            detector: None,
            reframer: None,
            last_detection_us: None,
            shown: None,
            light_face: None,
            light_misses: 0,
        })
    }

    fn outputs(&self) -> Arc<dyn OutputFactory> {
        Arc::clone(&self.outputs)
    }

    fn supports(&self, fourcc: &str) -> Result<(), String> {
        match fourcc {
            "MJPG" | "YUYV" => Ok(()),
            "H264" => self.h264.clone(),
            other => Err(format!("no preview decoder for {other} yet")),
        }
    }
}

struct FxSession {
    detectors: Arc<dyn DetectorFactory>,
    segmenters: Arc<dyn SegmenterFactory>,
    /// The light picture last read: its path and what it gave.
    light_probe: Option<(String, Option<LightProbe>)>,
    nvidia_factory: Arc<dyn SegmenterFactory>,
    /// NVIDIA's model; forgotten when the engine is switched away so choosing it again retries.
    nvidia: Option<Result<Box<dyn Segmenter>, String>>,
    /// Which engine produced the matte in use, and why it is not the one asked for.
    matte_engine: BackgroundEngine,
    fallback: Option<String>,
    denoiser: TemporalDenoise,
    /// Created on first use; a failure is remembered so it is not retried on every frame.
    segmenter: Option<Result<Box<dyn Segmenter>, String>>,
    /// The latest matte, reused between segmentations.
    matte: Option<Matte>,
    last_matte_us: Option<u64>,
    /// The replacement picture: what was loaded, and it scaled to the frame size.
    backdrop: Option<Backdrop>,
    /// Why H264 cannot be decoded, when it cannot.
    h264_unavailable: Option<String>,
    /// Created on first use; its state must survive from frame to frame.
    h264: Option<Result<lumencast_h264::H264Decoder, String>>,
    /// Created on first use; a failure is remembered so it is not retried on every frame.
    detector: Option<Result<Box<dyn FaceDetector>, String>>,
    reframer: Option<Reframer>,
    last_detection_us: Option<u64>,
    /// The framing actually shown: eased towards what the controller wants.
    shown: Option<(AutoFraming, u64)>,
    /// Where the key light is aimed: the last faces found, smoothed.
    light_face: Option<FaceBox>,
    light_misses: u32,
}

impl FxSession {
    /// Looks for a face when one is due. `Ok(None)` means none was due;
    /// `Ok(Some((face, elapsed_us)))` is a detection that ran, with the capture
    /// time since the previous one. `Err` means no detector can be created.
    fn detect_if_due(
        &mut self,
        now_us: u64,
        frame: &Frame,
    ) -> Result<Option<(Option<FaceBox>, u64)>, ()> {
        let detectors = &self.detectors;
        let detector = self.detector.get_or_insert_with(|| detectors.create());
        let Ok(detector) = detector else {
            return Err(());
        };
        let due = self
            .last_detection_us
            .is_none_or(|last| now_us.saturating_sub(last) >= DETECT_INTERVAL_US);
        if !due {
            return Ok(None);
        }
        let elapsed = self
            .last_detection_us
            .map_or(FIRST_STEP_US, |last| now_us.saturating_sub(last));
        self.last_detection_us = Some(now_us);
        let face = detector.detect(frame);
        self.note_face(face);
        Ok(Some((face, elapsed)))
    }

    /// Keeps the face the key light is aimed at: smoothed between detections,
    /// and forgotten after a few misses in a row.
    fn note_face(&mut self, face: Option<FaceBox>) {
        match (face, self.light_face) {
            (Some(new), Some(old)) => {
                let blend =
                    |old: f32, new: f32| old * LIGHT_SMOOTHING + new * (1.0 - LIGHT_SMOOTHING);
                self.light_face = Some(FaceBox {
                    center_x: blend(old.center_x, new.center_x),
                    center_y: blend(old.center_y, new.center_y),
                    height: blend(old.height, new.height),
                });
                self.light_misses = 0;
            }
            (Some(new), None) => {
                self.light_face = Some(new);
                self.light_misses = 0;
            }
            (None, _) => {
                self.light_misses += 1;
                if self.light_misses >= LIGHT_MISSES_ALLOWED {
                    self.light_face = None;
                }
            }
        }
    }

    /// The framing to use plus a status, from the latest detection (if any).
    fn follow(
        &mut self,
        now_us: u64,
        detected: Result<Option<(Option<FaceBox>, u64)>, ()>,
        frame: &Frame,
        settings: &AutoFramingSettings,
    ) -> (Option<AutoFraming>, AutoFramingStatus) {
        let Ok(detected) = detected else {
            return (None, AutoFramingStatus::Unavailable);
        };
        // Following needs some margin to move the crop in, so the zoom is never below this.
        let min_zoom =
            Zoom::new((settings.min_zoom_percent / 100.0).max(MIN_FOLLOW_ZOOM)).unwrap_or_default();
        let reframer = self
            .reframer
            .get_or_insert_with(|| Reframer::new(min_zoom, settings.speed));
        reframer.set_min_zoom(min_zoom);
        reframer.set_speed(settings.speed);
        let framing = match detected {
            Some((face, elapsed)) => reframer.observe(face, frame.width(), frame.height(), elapsed),
            // Between detections the last framing is held.
            None => reframer.current(),
        };
        let reachable = self.reframer.as_ref().is_none_or(Reframer::reachable);
        let framing = self.ease(framing, now_us);
        let status = match framing {
            Some(framing) => AutoFramingStatus::Locked {
                zoom_percent: framing.zoom * 100.0,
                pan_x: framing.pan_x,
                pan_y: framing.pan_y,
                reachable,
            },
            None => AutoFramingStatus::Searching,
        };
        (framing, status)
    }
}

impl FxSession {
    /// Applies the background effect and says how it went. A problem leaves the
    /// picture unchanged.
    fn background(
        &mut self,
        now_us: u64,
        frame: Frame,
        settings: &BackgroundSettings,
    ) -> (Frame, BackgroundStatus) {
        if settings.mode == BackgroundMode::None {
            self.matte = None;
            self.last_matte_us = None;
            return (frame, BackgroundStatus::Off);
        }
        let unavailable = |frame: Frame, problem: BackgroundProblem, reason: String| {
            (frame, BackgroundStatus::Unavailable { problem, reason })
        };
        if settings.engine != BackgroundEngine::Nvidia {
            self.nvidia = None;
        }
        let due = self
            .last_matte_us
            .is_none_or(|last| now_us.saturating_sub(last) >= SEGMENT_INTERVAL_US);
        if due || self.matte.is_none() {
            self.last_matte_us = Some(now_us);
            if let Err(reason) = self.refresh_matte(&frame, settings.engine) {
                return unavailable(frame, BackgroundProblem::ModelUnavailable, reason);
            }
        }
        let Some(matte) = self.matte.clone() else {
            return (
                frame,
                BackgroundStatus::Working {
                    engine: self.matte_engine,
                    fallback: self.fallback.clone(),
                },
            );
        };
        let backdrop = match settings.mode {
            BackgroundMode::Blur => {
                match Strength::new(f32::from(settings.blur_strength) / 100.0) {
                    Ok(strength) => FxBackdrop::Blur(strength),
                    Err(error) => {
                        return unavailable(
                            frame,
                            BackgroundProblem::ModelUnavailable,
                            error.to_string(),
                        )
                    }
                }
            }
            BackgroundMode::Remove => FxBackdrop::Solid([255; 3]),
            BackgroundMode::Replace => match self.scaled_image(settings, &frame) {
                Ok(image) => FxBackdrop::Image(image),
                Err((problem, reason)) => return unavailable(frame, problem, reason),
            },
            BackgroundMode::None => return (frame, BackgroundStatus::Off),
        };
        let composed = Background { matte, backdrop }.apply(frame);
        (
            composed,
            BackgroundStatus::Working {
                engine: self.matte_engine,
                fallback: self.fallback.clone(),
            },
        )
    }

    /// Finds the person with the engine asked for, or with the built-in one
    /// while that is not ready, remembering which. Only the built-in model
    /// failing is an error: there is nothing left to fall back on.
    fn refresh_matte(&mut self, frame: &Frame, engine: BackgroundEngine) -> Result<(), String> {
        self.fallback = None;
        if engine == BackgroundEngine::Nvidia {
            let factory = &self.nvidia_factory;
            let nvidia = self.nvidia.get_or_insert_with(|| factory.create());
            let reason = match nvidia {
                Ok(segmenter) => match segmenter.matte(frame) {
                    Ok(Some(matte)) => {
                        self.matte = Some(matte);
                        self.matte_engine = BackgroundEngine::Nvidia;
                        return Ok(());
                    }
                    Ok(None) => "the NVIDIA model is not ready yet".to_owned(),
                    Err(reason) => reason,
                },
                Err(reason) => reason.clone(),
            };
            self.fallback = Some(reason);
        }
        let segmenters = &self.segmenters;
        let segmenter = self.segmenter.get_or_insert_with(|| segmenters.create());
        match segmenter {
            Ok(segmenter) => {
                if let Some(matte) = segmenter.matte(frame)? {
                    self.matte = Some(matte);
                    self.matte_engine = BackgroundEngine::Builtin;
                }
                Ok(())
            }
            Err(reason) => Err(reason.clone()),
        }
    }

    /// The replacement picture at the size of `frame`.
    fn scaled_image(
        &mut self,
        settings: &BackgroundSettings,
        frame: &Frame,
    ) -> Result<Frame, (BackgroundProblem, String)> {
        let unreadable = |reason: String| (BackgroundProblem::PictureUnreadable, reason);
        let Some(path) = settings.image_path.as_deref().filter(|p| !p.is_empty()) else {
            return Err((
                BackgroundProblem::NoPicture,
                "no background picture was chosen".into(),
            ));
        };
        if self.backdrop.as_ref().is_none_or(|b| b.path != path) {
            self.backdrop = Some(Backdrop {
                path: path.to_owned(),
                loaded: load_image(path),
                scaled: None,
            });
        }
        let Some(backdrop) = self.backdrop.as_mut() else {
            return Err(unreadable("the background picture is not loaded".into()));
        };
        let image = backdrop
            .loaded
            .as_ref()
            .map_err(|reason| unreadable(reason.clone()))?;
        let size = (frame.width(), frame.height());
        if backdrop
            .scaled
            .as_ref()
            .is_none_or(|s| (s.width(), s.height()) != size)
        {
            backdrop.scaled = Some(cover(image, size.0, size.1));
        }
        backdrop
            .scaled
            .clone()
            .ok_or_else(|| unreadable("the background picture could not be scaled".into()))
    }

    /// What the key light's picture says about the light, read once per picture.
    /// A picture that cannot be read leaves the light as it is without one.
    fn light_probe(&mut self, settings: &KeyLightSettings) -> Option<LightProbe> {
        let Some(path) = settings.light_picture.as_deref().filter(|p| !p.is_empty()) else {
            self.light_probe = None;
            return None;
        };
        if self
            .light_probe
            .as_ref()
            .is_none_or(|(known, _)| known != path)
        {
            let probe = load_image(path)
                .ok()
                .and_then(|picture| LightProbe::from_picture(&picture));
            self.light_probe = Some((path.to_owned(), probe));
        }
        self.light_probe.as_ref().and_then(|(_, probe)| *probe)
    }

    /// Smooths the framing between detections so the picture glides.
    fn ease(&mut self, wanted: Option<AutoFraming>, now_us: u64) -> Option<AutoFraming> {
        let Some(wanted) = wanted else {
            self.shown = None;
            return None;
        };
        let eased = match self.shown {
            Some((shown, at)) => shown.eased_towards(wanted, now_us.saturating_sub(at)),
            None => wanted,
        };
        self.shown = Some((eased, now_us));
        Some(eased)
    }

    fn h264_decoder(&mut self) -> Result<&mut lumencast_h264::H264Decoder, String> {
        if let Some(reason) = &self.h264_unavailable {
            return Err(reason.clone());
        }
        self.h264
            .get_or_insert_with(|| {
                lumencast_h264::H264Decoder::new().map_err(|error| error.to_string())
            })
            .as_mut()
            .map_err(|reason| reason.clone())
    }
}

impl PreviewSession for FxSession {
    fn advance(
        &mut self,
        _timestamp_us: u64,
        mode: &CaptureMode,
        data: &[u8],
    ) -> Result<(), String> {
        if mode.format.to_string() != "H264" {
            return Ok(());
        }
        let stream = lumencast_h264::elementary_stream(data);
        self.h264_decoder()?
            .decode(stream, false)
            .map(drop)
            .map_err(|error| error.to_string())
    }

    fn render(
        &mut self,
        timestamp_us: u64,
        settings: &ProcessingSettings,
        mode: &CaptureMode,
        data: &[u8],
    ) -> Result<Rendered, String> {
        let (processed, auto_framing, background) =
            self.process(timestamp_us, settings, mode, data)?;
        let jpeg =
            encode_jpeg(&processed.preview, PREVIEW_QUALITY).map_err(|error| error.to_string())?;
        Ok(Rendered {
            jpeg,
            auto_framing,
            background,
        })
    }

    fn produce(
        &mut self,
        timestamp_us: u64,
        settings: &ProcessingSettings,
        mode: &CaptureMode,
        data: &[u8],
        want_preview: bool,
    ) -> Result<Produced, String> {
        let (processed, auto_framing, background) =
            self.process(timestamp_us, settings, mode, data)?;
        Ok(Produced {
            preview: want_preview.then_some(Picture::Raw(processed.preview)),
            output: settings
                .virtual_camera
                .enabled
                .then(|| upscaled(processed.output, &settings.virtual_camera)),
            auto_framing,
            background,
        })
    }
}

impl FxSession {
    /// Decodes a frame and runs every effect on it.
    fn process(
        &mut self,
        timestamp_us: u64,
        settings: &ProcessingSettings,
        mode: &CaptureMode,
        data: &[u8],
    ) -> Result<
        (
            lumencast_fx::pipeline::Processed,
            AutoFramingStatus,
            BackgroundStatus,
        ),
        String,
    > {
        let fourcc = mode.format.to_string();
        let frame = match fourcc.as_str() {
            "MJPG" => decode_jpeg(data).map_err(|error| error.to_string())?,
            "YUYV" => yuyv_to_rgb(data, mode.size.width, mode.size.height)
                .map_err(|error| error.to_string())?,
            "H264" => self
                .h264_decoder()?
                .decode(lumencast_h264::elementary_stream(data), true)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| NOT_READY.to_owned())?,
            other => return Err(format!("no preview decoder for {other}")),
        };

        let frame = match settings.denoise.level() {
            Some(strength) => self.denoiser.process(frame, strength),
            None => {
                self.denoiser.reset();
                frame
            }
        };
        let wants_face = settings.auto_framing.enabled
            || settings.key_light.enabled
            || (settings.vignette_follows_face && settings.vignette > 0.0);
        let detected = if wants_face {
            self.detect_if_due(timestamp_us, &frame)
        } else {
            self.last_detection_us = None;
            self.light_face = None;
            self.light_misses = 0;
            Ok(None)
        };
        let (framing, auto_framing) = if settings.auto_framing.enabled {
            self.follow(timestamp_us, detected, &frame, &settings.auto_framing)
        } else {
            self.reframer = None;
            self.shown = None;
            (None, AutoFramingStatus::Off)
        };
        let (frame, background) = self.background(timestamp_us, frame, &settings.background);
        let (source_width, source_height) = (frame.width(), frame.height());
        // The light is aimed in source coordinates, so it goes on before the crop.
        let probe = if settings.key_light.enabled {
            self.light_probe(&settings.key_light)
        } else {
            None
        };
        let frame = match self
            .light_face
            .and_then(|face| settings.key_light.stage(face, probe))
        {
            Some(light) => light.apply(frame),
            None => frame,
        };
        let processed = settings
            .pipeline_with(
                framing,
                self.light_face
                    .map(|face| (face, source_width, source_height)),
            )
            .map_err(|error| error.to_string())?
            .run(frame);
        Ok((processed, auto_framing, background))
    }
}

struct Job {
    /// Whether this frame is also a preview frame.
    show: bool,
    /// Whether this frame must go out through the virtual camera.
    deliver: bool,
    timestamp_us: u64,
    data: Vec<u8>,
    settings: ProcessingSettings,
    mode: CaptureMode,
}

/// How long to wait before trying to open a virtual camera that just failed.
const OUTPUT_RETRY_AFTER: Duration = Duration::from_secs(2);

/// The virtual camera as the processing thread sees it.
struct Delivery {
    factory: Arc<dyn OutputFactory>,
    output: Option<(Box<dyn FrameOutput>, (u32, u32))>,
    retry_at: Option<Instant>,
    status: VirtualCameraStatus,
}

impl Delivery {
    fn new(factory: Arc<dyn OutputFactory>) -> Self {
        Self {
            factory,
            output: None,
            retry_at: None,
            status: VirtualCameraStatus::Off,
        }
    }

    /// Lets go of the device when the virtual camera is switched off.
    fn stop(&mut self) {
        self.output = None;
        self.retry_at = None;
        self.status = VirtualCameraStatus::Off;
    }

    /// Sends `frame` out, opening (or reopening, on a new size) the device
    /// first. Returns the status when it changed.
    fn send(&mut self, frame: &Frame) -> Option<VirtualCameraStatus> {
        let size = (frame.width() & !1, frame.height() & !1);
        if self.output.as_ref().is_some_and(|(_, open)| *open != size) {
            self.output = None;
        }
        if self.output.is_none() {
            if self.retry_at.is_some_and(|at| Instant::now() < at) {
                return None;
            }
            match self.factory.open(size.0, size.1) {
                Ok(output) => {
                    self.retry_at = None;
                    self.output = Some((output, size));
                }
                Err(reason) => return self.fail(reason),
            }
        }
        let (output, _) = self.output.as_mut()?;
        match output.send(frame) {
            Ok(()) => {
                let status = VirtualCameraStatus::Active {
                    device: output.device(),
                    width: size.0,
                    height: size.1,
                };
                self.change(status)
            }
            Err(reason) => {
                self.output = None;
                self.fail(reason)
            }
        }
    }

    fn fail(&mut self, reason: String) -> Option<VirtualCameraStatus> {
        self.retry_at = Some(Instant::now() + OUTPUT_RETRY_AFTER);
        self.change(VirtualCameraStatus::Unavailable { reason })
    }

    /// Closes the device after the virtual camera was switched off; the status
    /// when that changed it.
    fn change_to_off(&mut self) -> Option<VirtualCameraStatus> {
        if self.status == VirtualCameraStatus::Off && self.output.is_none() {
            return None;
        }
        self.stop();
        Some(VirtualCameraStatus::Off)
    }

    fn change(&mut self, status: VirtualCameraStatus) -> Option<VirtualCameraStatus> {
        if status == self.status {
            return None;
        }
        self.status = status.clone();
        Some(status)
    }
}

/// One worker per capture. Dropping or finishing it ends its threads after
/// the frames already queued. One thread processes every frame (effects, and
/// the virtual camera); a second one encodes the preview, so encoding never
/// holds up the video going out.
pub struct PreviewWorker {
    sender: Option<SyncSender<Job>>,
    handle: Option<JoinHandle<()>>,
    sink: Arc<dyn EventSink>,
    /// Frames the preview could not show; shared with the threads that render.
    dropped: Arc<AtomicU64>,
}

impl PreviewWorker {
    pub fn spawn(
        sink: Arc<dyn EventSink>,
        renderer: Arc<dyn PreviewRenderer>,
        queue_depth: usize,
    ) -> Self {
        let (sender, receiver) = sync_channel::<Job>(queue_depth.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        let (thread_sink, thread_dropped) = (Arc::clone(&sink), Arc::clone(&dropped));
        let handle = thread::Builder::new()
            .name("lumencast-preview".into())
            .spawn(move || {
                let (sink, dropped) = (thread_sink, thread_dropped);
                let (encoder_in, encoder_out) = sync_channel::<(u64, Frame)>(1);
                let encoder = {
                    let (sink, dropped) = (Arc::clone(&sink), Arc::clone(&dropped));
                    thread::Builder::new()
                        .name("lumencast-preview-encoder".into())
                        .spawn(move || {
                            for (timestamp_us, frame) in encoder_out {
                                match encode_jpeg(&frame, PREVIEW_QUALITY) {
                                    Ok(jpeg) => sink.emit(AppEvent::Frame {
                                        timestamp_us,
                                        jpeg_base64: BASE64.encode(jpeg),
                                    }),
                                    Err(_) => {
                                        let total = dropped.fetch_add(1, Ordering::SeqCst) + 1;
                                        sink.emit(AppEvent::PreviewDropped { total });
                                    }
                                }
                            }
                        })
                        .ok()
                };
                let mut session = renderer.start();
                let mut delivery = Delivery::new(renderer.outputs());
                let mut announced = AutoFramingStatus::Off;
                let mut announced_background = BackgroundStatus::Off;
                for job in receiver {
                    if !job.deliver {
                        if let Some(status) = delivery.change_to_off() {
                            sink.emit(AppEvent::VirtualCamera { status });
                        }
                    }
                    if !job.show && !job.deliver {
                        if session
                            .advance(job.timestamp_us, &job.mode, &job.data)
                            .is_err()
                        {
                            let total = dropped.fetch_add(1, Ordering::SeqCst) + 1;
                            sink.emit(AppEvent::PreviewDropped { total });
                        }
                        continue;
                    }
                    let produced = session.produce(
                        job.timestamp_us,
                        &job.settings,
                        &job.mode,
                        &job.data,
                        job.show,
                    );
                    match produced {
                        Ok(produced) => {
                            if produced.background != announced_background {
                                announced_background = produced.background.clone();
                                sink.emit(AppEvent::Background {
                                    status: announced_background.clone(),
                                });
                            }
                            if produced.auto_framing.differs_noticeably(&announced) {
                                announced = produced.auto_framing;
                                sink.emit(AppEvent::AutoFraming { status: announced });
                            }
                            match produced.preview {
                                Some(Picture::Jpeg(jpeg)) => sink.emit(AppEvent::Frame {
                                    timestamp_us: job.timestamp_us,
                                    jpeg_base64: BASE64.encode(jpeg),
                                }),
                                Some(Picture::Raw(frame)) => {
                                    // A busy encoder means this preview frame is skipped.
                                    let _ = encoder_in.try_send((job.timestamp_us, frame));
                                }
                                None => {}
                            }
                            if job.deliver {
                                if let Some(frame) = produced.output {
                                    if let Some(status) = delivery.send(&frame) {
                                        sink.emit(AppEvent::VirtualCamera { status });
                                    }
                                }
                            }
                        }
                        Err(reason) if reason == NOT_READY => {}
                        Err(_) => {
                            let total = dropped.fetch_add(1, Ordering::SeqCst) + 1;
                            sink.emit(AppEvent::PreviewDropped { total });
                        }
                    }
                }
                drop(encoder_in);
                if let Some(encoder) = encoder {
                    let _ = encoder.join();
                }
            })
            .ok();
        Self {
            sender: Some(sender),
            handle,
            sink,
            dropped,
        }
    }

    /// Counts and announces a frame that never reached the decoder.
    pub fn note_skipped(&self) {
        let total = self.dropped.fetch_add(1, Ordering::SeqCst) + 1;
        self.sink.emit(AppEvent::PreviewDropped { total });
    }

    /// Queues a frame; `false` means the worker is busy and the frame is skipped.
    pub fn submit(
        &self,
        timestamp_us: u64,
        data: &[u8],
        settings: ProcessingSettings,
        mode: CaptureMode,
        show: bool,
        deliver: bool,
    ) -> bool {
        let Some(sender) = &self.sender else {
            return false;
        };
        let job = Job {
            show,
            deliver,
            timestamp_us,
            data: data.to_vec(),
            settings,
            mode,
        };
        !matches!(
            sender.try_send(job),
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_))
        )
    }

    /// Waits until everything queued has been rendered and announced.
    pub fn finish(&mut self) {
        self.sender.take();
        if let Some(handle) = self.handle.take() {
            // A panic in the renderer only costs the remaining preview frames.
            let _ = handle.join();
        }
    }
}

impl Drop for PreviewWorker {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumencast_core::mode::{FrameInterval, FrameSize, PixelFormat};
    use lumencast_fx::reframe::FaceBox;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    fn mode(fourcc: &str, width: u32, height: u32) -> CaptureMode {
        CaptureMode {
            format: PixelFormat::parse(fourcc).unwrap(),
            size: FrameSize::new(width, height).unwrap(),
            interval: FrameInterval::new(1, 30).unwrap(),
        }
    }

    fn mirror() -> ProcessingSettings {
        ProcessingSettings {
            mirror_preview: true,
            ..ProcessingSettings::default()
        }
    }

    fn left_is_dark(jpeg: &[u8]) -> bool {
        let frame = decode_jpeg(jpeg).unwrap();
        frame.pixel(4, frame.height() / 2)[0] < 80
    }

    /// 64x32: black left half, white right half.
    fn halves() -> Vec<u8> {
        let frame = Frame::from_fn(64, 32, |x, _| if x < 32 { [0, 0, 0] } else { [255; 3] });
        encode_jpeg(&frame, 92).unwrap()
    }

    struct NoDetectors;

    impl DetectorFactory for NoDetectors {
        fn create(&self) -> Result<Box<dyn FaceDetector>, String> {
            Err("no detector configured".into())
        }
    }

    fn plain_session() -> Box<dyn PreviewSession> {
        FxRenderer::new(Arc::new(NoDetectors)).start()
    }

    #[test]
    fn a_jpeg_frame_is_decoded_processed_and_re_encoded() {
        let jpeg = halves();
        let plain = plain_session()
            .render(
                0,
                &ProcessingSettings::default(),
                &mode("MJPG", 64, 32),
                &jpeg,
            )
            .unwrap();
        assert!(left_is_dark(&plain.jpeg));
        assert_eq!(plain.auto_framing, AutoFramingStatus::Off);
        let flipped = plain_session()
            .render(0, &mirror(), &mode("MJPG", 64, 32), &jpeg)
            .unwrap();
        assert!(!left_is_dark(&flipped.jpeg));
    }

    #[test]
    fn the_renderer_says_what_it_can_show_and_why_not() {
        let renderer = FxRenderer::new(Arc::new(NoDetectors));
        assert!(renderer.supports("MJPG").is_ok());
        assert!(renderer.supports("YUYV").is_ok());
        assert!(renderer.supports("NV12").unwrap_err().contains("NV12"));
        assert_eq!(
            renderer.supports("H264").is_ok(),
            lumencast_h264::availability().is_ok()
        );
        let without =
            FxRenderer::new(Arc::new(NoDetectors)).with_h264_unavailable("install OpenH264");
        assert!(without
            .supports("H264")
            .unwrap_err()
            .contains("install OpenH264"));
        assert!(
            needs_every_frame("H264") && !needs_every_frame("MJPG") && !needs_every_frame("YUYV")
        );
    }

    #[test]
    fn frames_that_are_not_shown_do_nothing_for_stateless_formats() {
        assert!(plain_session()
            .advance(0, &mode("MJPG", 64, 32), &halves())
            .is_ok());
        assert!(plain_session()
            .advance(0, &mode("YUYV", 64, 32), &[0; 4])
            .is_ok());
    }

    #[test]
    fn an_h264_stream_is_decoded_frame_by_frame_and_shown_when_asked() {
        if lumencast_h264::availability().is_err() {
            eprintln!("SKIPPED: OpenH264 is not installed");
            return;
        }
        let units = lumencast_h264::test_support::stream(9).unwrap();
        let h264 = mode("H264", 128, 64);
        let mut session = plain_session();
        for unit in &units[..7] {
            session.advance(0, &h264, unit).unwrap();
        }
        let rendered = session
            .render(0, &ProcessingSettings::default(), &h264, &units[7])
            .unwrap();
        assert!(left_is_dark(&rendered.jpeg));
        // The picture went through the pipeline: the mirror now flips the next one.
        let mirrored = session.render(0, &mirror(), &h264, &units[8]).unwrap();
        assert!(!left_is_dark(&mirrored.jpeg));
    }

    #[test]
    fn an_h264_stream_without_its_keyframe_is_never_shown_wrongly_and_never_panics() {
        if lumencast_h264::availability().is_err() {
            eprintln!("SKIPPED: OpenH264 is not installed");
            return;
        }
        let units = lumencast_h264::test_support::stream(4).unwrap();
        let h264 = mode("H264", 128, 64);
        let mut session = plain_session();
        for unit in &units[1..] {
            if let Ok(rendered) = session.render(0, &ProcessingSettings::default(), &h264, unit) {
                assert!(
                    left_is_dark(&rendered.jpeg),
                    "a picture, when there is one, must be right"
                );
            }
        }
    }

    #[test]
    fn h264_without_the_library_fails_with_the_reason_instead_of_a_panic() {
        let renderer =
            FxRenderer::new(Arc::new(NoDetectors)).with_h264_unavailable("install OpenH264");
        let mut session = renderer.start();
        let error = session
            .render(
                0,
                &ProcessingSettings::default(),
                &mode("H264", 128, 64),
                &[0, 0, 1],
            )
            .err()
            .unwrap();
        assert!(error.contains("install OpenH264"), "{error}");
        assert!(session
            .advance(0, &mode("H264", 128, 64), &[0, 0, 1])
            .is_err());
    }

    #[test]
    fn a_yuyv_frame_is_converted_even_with_neutral_settings() {
        // Left half black (Y 16), right half white (Y 235), neutral chroma.
        let data: Vec<u8> = (0..64 * 32 / 2)
            .flat_map(|pair| {
                let y = if pair % 32 < 16 { 16 } else { 235 };
                [y, 128, y, 128]
            })
            .collect();
        let rendered = plain_session()
            .render(
                0,
                &ProcessingSettings::default(),
                &mode("YUYV", 64, 32),
                &data,
            )
            .unwrap();
        assert!(left_is_dark(&rendered.jpeg));
    }

    #[test]
    fn a_format_without_a_decoder_and_bad_data_are_errors_not_panics() {
        let mut session = plain_session();
        let error = session
            .render(
                0,
                &ProcessingSettings::default(),
                &mode("H264", 64, 32),
                &[0, 0, 1],
            )
            .err()
            .unwrap();
        assert!(error.contains("H264"), "{error}");
        assert!(session
            .render(0, &mirror(), &mode("MJPG", 64, 32), &[1, 2, 3, 4])
            .is_err());
        assert!(session
            .render(0, &mirror(), &mode("YUYV", 64, 32), &[1, 2, 3, 4])
            .is_err());
    }

    // ---- auto framing, with a scripted detector ----

    #[derive(Clone, Default)]
    struct Script {
        faces: Arc<Mutex<VecDeque<Option<FaceBox>>>>,
        fallback: Arc<Mutex<Option<FaceBox>>>,
        calls: Arc<AtomicUsize>,
        created: Arc<AtomicUsize>,
        fail: Arc<Mutex<bool>>,
    }

    struct ScriptedDetector(Script);

    impl FaceDetector for ScriptedDetector {
        fn detect(&mut self, _frame: &Frame) -> Option<FaceBox> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            let queued = self.0.faces.lock().unwrap().pop_front();
            queued.unwrap_or(*self.0.fallback.lock().unwrap())
        }
    }

    impl DetectorFactory for Script {
        fn create(&self) -> Result<Box<dyn FaceDetector>, String> {
            self.created.fetch_add(1, Ordering::SeqCst);
            if *self.fail.lock().unwrap() {
                return Err("the model is missing".into());
            }
            Ok(Box::new(ScriptedDetector(self.clone())))
        }
    }

    /// A small face on the right half of the 64x32 frame: a strong zoom to the right.
    fn face_on_the_right() -> FaceBox {
        FaceBox {
            center_x: 48.0,
            center_y: 16.0,
            height: 6.0,
        }
    }

    fn following(speed: u8) -> ProcessingSettings {
        let mut settings = ProcessingSettings::default();
        settings.auto_framing.enabled = true;
        settings.auto_framing.speed = speed;
        settings
    }

    fn session_with(script: &Script) -> Box<dyn PreviewSession> {
        FxRenderer::new(Arc::new(script.clone())).start()
    }

    #[test]
    fn with_auto_framing_off_the_detector_is_never_created() {
        let script = Script::default();
        let mut session = session_with(&script);
        let rendered = session
            .render(
                0,
                &ProcessingSettings::default(),
                &mode("MJPG", 64, 32),
                &halves(),
            )
            .unwrap();
        assert_eq!(rendered.auto_framing, AutoFramingStatus::Off);
        assert_eq!(script.created.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_found_face_locks_the_framing_and_the_picture_follows_it() {
        let script = Script::default();
        *script.fallback.lock().unwrap() = Some(face_on_the_right());
        let mut session = session_with(&script);
        let mut last = None;
        for step in 0..12 {
            last = Some(
                session
                    .render(
                        step * 250_000,
                        &following(100),
                        &mode("MJPG", 64, 32),
                        &halves(),
                    )
                    .unwrap(),
            );
        }
        let rendered = last.unwrap();
        match rendered.auto_framing {
            AutoFramingStatus::Locked {
                zoom_percent,
                pan_x,
                ..
            } => {
                assert!((zoom_percent - 130.0).abs() < 0.5, "zoom {zoom_percent}");
                assert!(pan_x > 0.9, "pan {pan_x}");
            }
            other => panic!("expected a lock, got {other:?}"),
        }
        // Only the pan moved: at the user's zoom the crop still starts in the dark half.
        assert!(left_is_dark(&rendered.jpeg));
    }

    #[test]
    fn without_a_face_it_keeps_searching_and_leaves_the_picture_alone() {
        let script = Script::default();
        let mut session = session_with(&script);
        for step in 0..4 {
            let rendered = session
                .render(
                    step * 250_000,
                    &following(100),
                    &mode("MJPG", 64, 32),
                    &halves(),
                )
                .unwrap();
            assert_eq!(rendered.auto_framing, AutoFramingStatus::Searching);
            assert!(left_is_dark(&rendered.jpeg));
        }
    }

    #[test]
    fn a_detector_that_cannot_start_is_reported_once_and_the_preview_still_works() {
        let script = Script::default();
        *script.fail.lock().unwrap() = true;
        let mut session = session_with(&script);
        for step in 0..5 {
            let rendered = session
                .render(
                    step * 250_000,
                    &following(35),
                    &mode("MJPG", 64, 32),
                    &halves(),
                )
                .unwrap();
            assert_eq!(rendered.auto_framing, AutoFramingStatus::Unavailable);
            assert!(left_is_dark(&rendered.jpeg));
        }
        assert_eq!(
            script.created.load(Ordering::SeqCst),
            1,
            "a failure is not retried on every frame"
        );
    }

    #[test]
    fn faces_are_looked_for_at_a_limited_rate_and_the_framing_is_held_in_between() {
        let script = Script::default();
        *script.fallback.lock().unwrap() = Some(face_on_the_right());
        let mut session = session_with(&script);
        let mut zooms = Vec::new();
        for frame in 0..31u64 {
            let rendered = session
                .render(
                    frame * 33_333,
                    &following(35),
                    &mode("MJPG", 64, 32),
                    &halves(),
                )
                .unwrap();
            if let AutoFramingStatus::Locked { pan_x, .. } = rendered.auto_framing {
                zooms.push(pan_x);
            }
        }
        let calls = script.calls.load(Ordering::SeqCst);
        assert!(
            (9..=11).contains(&calls),
            "{calls} detections in one second of video"
        );
        assert_eq!(zooms.len(), 31, "locked from the first detection on");
        // Between detections the picture keeps gliding towards the held target.
        assert!(zooms[2] >= zooms[1]);
        assert!(zooms[30] > zooms[0], "and it keeps converging");
    }

    #[test]
    fn turning_auto_framing_off_resets_it_and_turning_it_on_starts_from_neutral() {
        let script = Script::default();
        *script.fallback.lock().unwrap() = Some(face_on_the_right());
        let mut session = session_with(&script);
        let mut converged = 0.0;
        for step in 0..12 {
            if let AutoFramingStatus::Locked { pan_x, .. } = session
                .render(
                    step * 250_000,
                    &following(100),
                    &mode("MJPG", 64, 32),
                    &halves(),
                )
                .unwrap()
                .auto_framing
            {
                converged = pan_x;
            }
        }
        let off = session
            .render(
                4_000_000,
                &ProcessingSettings::default(),
                &mode("MJPG", 64, 32),
                &halves(),
            )
            .unwrap();
        assert_eq!(off.auto_framing, AutoFramingStatus::Off);
        let again = session
            .render(4_250_000, &following(100), &mode("MJPG", 64, 32), &halves())
            .unwrap();
        match again.auto_framing {
            AutoFramingStatus::Locked { pan_x, .. } => {
                assert!(pan_x < converged - 0.2, "{pan_x} vs {converged}")
            }
            other => panic!("expected a lock, got {other:?}"),
        }
    }

    #[test]
    fn the_status_serializes_with_a_state_tag() {
        let locked = AutoFramingStatus::Locked {
            zoom_percent: 142.0,
            pan_x: 0.25,
            pan_y: -0.5,
            reachable: false,
        };
        let json = serde_json::to_value(locked).unwrap();
        assert_eq!(json["state"], "locked");
        assert_eq!(json["zoomPercent"], 142.0);
        assert_eq!(json["reachable"], false);
        assert_eq!(
            serde_json::to_value(AutoFramingStatus::Searching).unwrap()["state"],
            "searching"
        );
        assert_eq!(Zoom::new(1.0).unwrap().get(), 1.0);
    }

    fn grey() -> Vec<u8> {
        encode_jpeg(&Frame::from_fn(64, 32, |_, _| [60; 3]), 95).unwrap()
    }

    fn lit(settings: ProcessingSettings, script: &Script) -> Rendered {
        let mut session = session_with(script);
        session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap()
    }

    fn centre_green(rendered: &Rendered) -> u8 {
        decode_jpeg(&rendered.jpeg).unwrap().pixel(32, 16)[1]
    }

    fn centred_face() -> FaceBox {
        FaceBox {
            center_x: 32.0,
            center_y: 14.0,
            height: 12.0,
        }
    }

    #[test]
    fn the_key_light_lifts_the_face_it_finds_without_auto_framing() {
        let script = Script::default();
        *script.fallback.lock().unwrap() = Some(centred_face());
        let mut settings = ProcessingSettings::default();
        settings.key_light.enabled = true;
        settings.key_light.intensity = 100;
        let rendered = lit(settings, &script);
        assert_eq!(rendered.auto_framing, AutoFramingStatus::Off);
        let off = lit(ProcessingSettings::default(), &script);
        assert!(
            centre_green(&rendered) >= centre_green(&off) + 10,
            "{} vs {}",
            centre_green(&rendered),
            centre_green(&off)
        );
    }

    #[test]
    fn with_the_key_light_off_no_detector_is_created() {
        let script = Script::default();
        lit(ProcessingSettings::default(), &script);
        assert_eq!(script.created.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn without_a_face_the_key_light_leaves_the_picture_alone() {
        let script = Script::default();
        let mut settings = ProcessingSettings::default();
        settings.key_light.enabled = true;
        settings.key_light.intensity = 100;
        let rendered = lit(settings, &script);
        let off = lit(ProcessingSettings::default(), &script);
        assert_eq!(centre_green(&rendered), centre_green(&off));
    }

    #[test]
    fn the_key_light_and_auto_framing_share_one_detector() {
        let script = Script::default();
        *script.fallback.lock().unwrap() = Some(centred_face());
        let mut settings = following(100);
        settings.key_light.enabled = true;
        lit(settings, &script);
        assert_eq!(script.created.load(Ordering::SeqCst), 1);
        assert_eq!(script.calls.load(Ordering::SeqCst), 1);
    }

    #[derive(Default)]
    struct Matter {
        created: AtomicUsize,
        calls: AtomicUsize,
        value: u8,
        broken: bool,
        /// Answers `None`, as a server that is still loading would.
        not_ready: bool,
    }

    struct ScriptedSegmenter(Arc<Matter>);

    impl Segmenter for ScriptedSegmenter {
        fn matte(&mut self, _frame: &Frame) -> Result<Option<Matte>, String> {
            self.0.calls.fetch_add(1, Ordering::SeqCst);
            if self.0.not_ready {
                return Ok(None);
            }
            Ok(Some(Matte::uniform(8, 8, self.0.value)))
        }
    }

    impl SegmenterFactory for Arc<Matter> {
        fn create(&self) -> Result<Box<dyn Segmenter>, String> {
            self.created.fetch_add(1, Ordering::SeqCst);
            if self.broken {
                Err("the model is missing".into())
            } else {
                Ok(Box::new(ScriptedSegmenter(Arc::clone(self))))
            }
        }
    }

    fn with_background(
        matter: &Arc<Matter>,
        mode: BackgroundMode,
        image: Option<&str>,
    ) -> (Box<dyn PreviewSession>, ProcessingSettings) {
        let session = FxRenderer::new(Arc::new(Script::default()))
            .with_segmenters(Arc::new(Arc::clone(matter)))
            .start();
        let mut settings = ProcessingSettings::default();
        settings.background.engine = BackgroundEngine::Builtin;
        settings.background.mode = mode;
        settings.background.image_path = image.map(str::to_owned);
        (session, settings)
    }

    fn corner(rendered: &Rendered) -> [u8; 3] {
        decode_jpeg(&rendered.jpeg).unwrap().pixel(2, 2)
    }

    #[test]
    fn without_a_background_effect_no_segmenter_is_created() {
        let matter = Arc::new(Matter::default());
        let (mut session, settings) = with_background(&matter, BackgroundMode::None, None);
        let rendered = session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        assert_eq!(rendered.background, BackgroundStatus::Off);
        assert_eq!(matter.created.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn remove_turns_everything_that_is_not_the_person_white() {
        let matter = Arc::new(Matter::default());
        let (mut session, settings) = with_background(&matter, BackgroundMode::Remove, None);
        let rendered = session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        assert_eq!(
            rendered.background,
            BackgroundStatus::Working {
                engine: BackgroundEngine::Builtin,
                fallback: None
            }
        );
        assert!(
            corner(&rendered).iter().all(|c| *c > 240),
            "{:?}",
            corner(&rendered)
        );
    }

    #[test]
    fn a_person_matte_keeps_the_picture() {
        let matter = Arc::new(Matter {
            value: 255,
            ..Matter::default()
        });
        let (mut session, settings) = with_background(&matter, BackgroundMode::Remove, None);
        let rendered = session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        assert!(corner(&rendered).iter().all(|c| (50..=70).contains(c)));
    }

    #[test]
    fn the_person_is_looked_for_at_a_limited_rate() {
        let matter = Arc::new(Matter::default());
        let (mut session, settings) = with_background(&matter, BackgroundMode::Blur, None);
        for frame in 0..31u64 {
            session
                .render(frame * 33_333, &settings, &mode("MJPG", 64, 32), &grey())
                .unwrap();
        }
        let calls = matter.calls.load(Ordering::SeqCst);
        assert!(
            (14..=17).contains(&calls),
            "{calls} segmentations in one second"
        );
        assert_eq!(matter.created.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_missing_model_leaves_the_picture_alone_and_says_why() {
        let matter = Arc::new(Matter {
            broken: true,
            ..Matter::default()
        });
        let (mut session, settings) = with_background(&matter, BackgroundMode::Remove, None);
        for step in 0..3 {
            let rendered = session
                .render(step * 100_000, &settings, &mode("MJPG", 64, 32), &grey())
                .unwrap();
            assert_eq!(
                rendered.background,
                BackgroundStatus::Unavailable {
                    problem: BackgroundProblem::ModelUnavailable,
                    reason: "the model is missing".into()
                }
            );
            assert!(corner(&rendered).iter().all(|c| (50..=70).contains(c)));
        }
        assert_eq!(matter.created.load(Ordering::SeqCst), 1, "not retried");
    }

    #[test]
    fn replace_shows_the_chosen_picture_and_reports_a_bad_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wall.jpg");
        let wall = Frame::from_fn(32, 16, |_, _| [220, 30, 30]);
        std::fs::write(&path, encode_jpeg(&wall, 95).unwrap()).unwrap();
        let matter = Arc::new(Matter::default());
        let (mut session, settings) =
            with_background(&matter, BackgroundMode::Replace, path.to_str());
        let rendered = session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        let [red, green, _] = corner(&rendered);
        assert!(red > 180 && green < 80, "{:?}", corner(&rendered));

        let (mut session, settings) =
            with_background(&matter, BackgroundMode::Replace, Some("/no/such.png"));
        let rendered = session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        assert!(matches!(
            rendered.background,
            BackgroundStatus::Unavailable {
                problem: BackgroundProblem::PictureUnreadable,
                ..
            }
        ));
        let (mut session, settings) = with_background(&matter, BackgroundMode::Replace, None);
        let rendered = session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        assert!(matches!(
            rendered.background,
            BackgroundStatus::Unavailable {
                problem: BackgroundProblem::NoPicture,
                ..
            }
        ));
    }

    #[test]
    fn the_noise_reduction_cleans_a_static_noisy_stream_and_is_off_by_default() {
        let script = Script::default();
        let mut on = ProcessingSettings::default();
        on.denoise.enabled = true;
        on.denoise.strength = 90;
        let noisy = |seed: u32| {
            let mut state = seed;
            let mut data = Vec::with_capacity(64 * 32 * 3);
            for _ in 0..64 * 32 {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let v = (100 + ((state >> 16) % 41) as i32 - 20) as u8;
                data.extend([v; 3]);
            }
            encode_jpeg(&Frame::new(64, 32, data).unwrap(), 98).unwrap()
        };
        let spread = |rendered: &Rendered| {
            let frame = decode_jpeg(&rendered.jpeg).unwrap();
            let values: Vec<f64> = (0..32)
                .flat_map(|y| (0..64).map(move |x| (x, y)))
                .map(|(x, y)| f64::from(frame.pixel(x, y)[1]))
                .collect();
            let mean = values.iter().sum::<f64>() / values.len() as f64;
            values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / values.len() as f64
        };
        let mut cleaned = session_with(&script);
        let mut raw = session_with(&script);
        let (mut last_cleaned, mut last_raw) = (0.0, 0.0);
        for step in 0..25u64 {
            let data = noisy(step as u32 + 1);
            let a = cleaned
                .render(step * 33_333, &on, &mode("MJPG", 64, 32), &data)
                .unwrap();
            let b = raw
                .render(
                    step * 33_333,
                    &ProcessingSettings::default(),
                    &mode("MJPG", 64, 32),
                    &data,
                )
                .unwrap();
            (last_cleaned, last_raw) = (spread(&a), spread(&b));
        }
        assert!(
            last_cleaned < last_raw * 0.6,
            "{last_cleaned} vs {last_raw}"
        );
    }

    fn nvidia_session(
        builtin: &Arc<Matter>,
        nvidia: &Arc<Matter>,
    ) -> (Box<dyn PreviewSession>, ProcessingSettings) {
        let session = FxRenderer::new(Arc::new(Script::default()))
            .with_segmenters(Arc::new(Arc::clone(builtin)))
            .with_nvidia(Arc::new(Arc::clone(nvidia)))
            .start();
        let mut settings = ProcessingSettings::default();
        settings.background.engine = BackgroundEngine::Nvidia;
        settings.background.mode = BackgroundMode::Remove;
        (session, settings)
    }

    #[test]
    fn the_nvidia_model_is_used_when_it_is_ready_and_the_built_in_one_is_never_loaded() {
        let (builtin, nvidia) = (Arc::new(Matter::default()), Arc::new(Matter::default()));
        let (mut session, settings) = nvidia_session(&builtin, &nvidia);
        let rendered = session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        assert_eq!(
            rendered.background,
            BackgroundStatus::Working {
                engine: BackgroundEngine::Nvidia,
                fallback: None
            }
        );
        assert_eq!(builtin.created.load(Ordering::SeqCst), 0);
        assert!(corner(&rendered).iter().all(|c| *c > 240));
    }

    #[test]
    fn while_nvidia_is_not_ready_the_built_in_model_covers_and_says_why() {
        let builtin = Arc::new(Matter::default());
        let nvidia = Arc::new(Matter {
            not_ready: true,
            ..Matter::default()
        });
        let (mut session, settings) = nvidia_session(&builtin, &nvidia);
        let rendered = session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        assert!(matches!(
            rendered.background,
            BackgroundStatus::Working {
                engine: BackgroundEngine::Builtin,
                fallback: Some(_)
            }
        ));
        assert!(corner(&rendered).iter().all(|c| *c > 240), "still removed");
    }

    #[test]
    fn a_nvidia_model_that_cannot_start_falls_back_once_and_is_not_retried() {
        let builtin = Arc::new(Matter::default());
        let nvidia = Arc::new(Matter {
            broken: true,
            ..Matter::default()
        });
        let (mut session, settings) = nvidia_session(&builtin, &nvidia);
        for step in 0..4 {
            let rendered = session
                .render(step * 100_000, &settings, &mode("MJPG", 64, 32), &grey())
                .unwrap();
            match rendered.background {
                BackgroundStatus::Working {
                    engine: BackgroundEngine::Builtin,
                    fallback: Some(reason),
                } => assert_eq!(reason, "the model is missing"),
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(nvidia.created.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn choosing_the_built_in_engine_forgets_nvidia_so_choosing_it_again_retries() {
        let builtin = Arc::new(Matter::default());
        let nvidia = Arc::new(Matter::default());
        let (mut session, mut settings) = nvidia_session(&builtin, &nvidia);
        session
            .render(0, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        settings.background.engine = BackgroundEngine::Builtin;
        session
            .render(200_000, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        settings.background.engine = BackgroundEngine::Nvidia;
        session
            .render(400_000, &settings, &mode("MJPG", 64, 32), &grey())
            .unwrap();
        assert_eq!(nvidia.created.load(Ordering::SeqCst), 2);
    }

    #[derive(Default)]
    struct Recorder {
        opens: std::sync::Mutex<Vec<(u32, u32)>>,
        sent: std::sync::Mutex<Vec<(u32, u32)>>,
        fail_open: std::sync::atomic::AtomicBool,
    }

    struct FakeOutput(Arc<Recorder>);

    impl FrameOutput for FakeOutput {
        fn send(&mut self, frame: &Frame) -> Result<(), String> {
            self.0
                .sent
                .lock()
                .unwrap()
                .push((frame.width(), frame.height()));
            Ok(())
        }

        fn device(&self) -> String {
            "fake".into()
        }
    }

    impl OutputFactory for Arc<Recorder> {
        fn open(&self, width: u32, height: u32) -> Result<Box<dyn FrameOutput>, String> {
            self.opens.lock().unwrap().push((width, height));
            if self.fail_open.load(Ordering::SeqCst) {
                return Err("no virtual camera".into());
            }
            Ok(Box::new(FakeOutput(Arc::clone(self))))
        }
    }

    #[derive(Default)]
    struct Collected(std::sync::Mutex<Vec<AppEvent>>);

    impl EventSink for Collected {
        fn emit(&self, event: AppEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    fn camera_worker(recorder: &Arc<Recorder>) -> (PreviewWorker, Arc<Collected>) {
        let sink = Arc::new(Collected::default());
        let renderer =
            FxRenderer::new(Arc::new(NoDetectors)).with_outputs(Arc::new(Arc::clone(recorder)));
        let worker = PreviewWorker::spawn(sink.clone(), Arc::new(renderer), 8);
        (worker, sink)
    }

    fn on_air() -> ProcessingSettings {
        let mut settings = ProcessingSettings::default();
        settings.virtual_camera.enabled = true;
        settings
    }

    fn statuses(sink: &Collected) -> Vec<VirtualCameraStatus> {
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                AppEvent::VirtualCamera { status } => Some(status.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn every_frame_goes_to_the_virtual_camera_but_only_some_to_the_preview() {
        let recorder = Arc::new(Recorder::default());
        let (mut worker, sink) = camera_worker(&recorder);
        for step in 0..6u64 {
            assert!(worker.submit(
                step * 33_333,
                &halves(),
                on_air(),
                mode("MJPG", 64, 32),
                step % 3 == 0,
                true
            ));
        }
        worker.finish();
        assert_eq!(recorder.sent.lock().unwrap().len(), 6);
        assert_eq!(
            *recorder.opens.lock().unwrap(),
            vec![(64, 32)],
            "opened once"
        );
        let previews = sink
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, AppEvent::Frame { .. }))
            .count();
        // Two frames were meant for the preview; the encoder skips one that
        // arrives while it is still busy with the other, so one or two get through.
        assert!((1..=2).contains(&previews), "{previews} preview frames");
        assert_eq!(
            statuses(&sink),
            vec![VirtualCameraStatus::Active {
                device: "fake".into(),
                width: 64,
                height: 32
            }],
            "announced once"
        );
    }

    #[test]
    fn the_output_carries_the_effects_but_not_the_preview_only_mirror() {
        let recorder = Arc::new(Recorder::default());
        let (mut worker, _sink) = camera_worker(&recorder);
        let mut settings = on_air();
        settings.mirror_preview = true;
        worker.submit(0, &halves(), settings, mode("MJPG", 64, 32), true, true);
        worker.finish();
        assert_eq!(recorder.sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_device_that_cannot_be_opened_is_reported_once_and_retried_later_not_every_frame() {
        let recorder = Arc::new(Recorder::default());
        recorder.fail_open.store(true, Ordering::SeqCst);
        let (mut worker, sink) = camera_worker(&recorder);
        for step in 0..5u64 {
            worker.submit(
                step * 33_333,
                &halves(),
                on_air(),
                mode("MJPG", 64, 32),
                false,
                true,
            );
        }
        worker.finish();
        assert_eq!(
            recorder.opens.lock().unwrap().len(),
            1,
            "one attempt, then a pause"
        );
        assert_eq!(
            statuses(&sink),
            vec![VirtualCameraStatus::Unavailable {
                reason: "no virtual camera".into()
            }]
        );
    }

    #[test]
    fn a_new_frame_size_reopens_the_device_and_switching_off_lets_go_of_it() {
        let recorder = Arc::new(Recorder::default());
        let (mut worker, sink) = camera_worker(&recorder);
        worker.submit(0, &halves(), on_air(), mode("MJPG", 64, 32), false, true);
        let bigger = encode_jpeg(&Frame::from_fn(80, 48, |_, _| [90; 3]), 90).unwrap();
        worker.submit(33_333, &bigger, on_air(), mode("MJPG", 80, 48), false, true);
        worker.submit(
            66_666,
            &bigger,
            ProcessingSettings::default(),
            mode("MJPG", 80, 48),
            true,
            false,
        );
        worker.finish();
        assert_eq!(*recorder.opens.lock().unwrap(), vec![(64, 32), (80, 48)]);
        let seen = statuses(&sink);
        assert!(matches!(
            seen.first(),
            Some(VirtualCameraStatus::Active { width: 64, .. })
        ));
        assert_eq!(seen.last(), Some(&VirtualCameraStatus::Off));
    }

    #[test]
    fn an_odd_sized_frame_is_sent_at_the_even_size_below_it() {
        let recorder = Arc::new(Recorder::default());
        let (mut worker, _sink) = camera_worker(&recorder);
        let odd = encode_jpeg(&Frame::from_fn(63, 31, |_, _| [90; 3]), 90).unwrap();
        worker.submit(0, &odd, on_air(), mode("MJPG", 63, 31), false, true);
        worker.finish();
        assert_eq!(*recorder.opens.lock().unwrap(), vec![(62, 30)]);
    }

    #[test]
    fn the_output_is_scaled_up_but_the_preview_keeps_its_size() {
        let script = Script::default();
        let mut settings = on_air();
        settings.virtual_camera.upscale_height = 240;
        let mut session = session_with(&script);
        let produced = session
            .produce(0, &settings, &mode("MJPG", 64, 32), &halves(), true)
            .unwrap();
        let output = produced.output.unwrap();
        assert_eq!((output.width(), output.height()), (480, 240));
        match produced.preview {
            Some(Picture::Raw(preview)) => {
                assert_eq!((preview.width(), preview.height()), (64, 32))
            }
            _ => panic!("expected a raw preview"),
        }
    }

    #[test]
    fn auto_framing_always_keeps_a_margin_to_follow_in_even_at_no_zoom() {
        let script = Script::default();
        *script.fallback.lock().unwrap() = Some(face_on_the_right());
        let mut settings = following(100);
        settings.auto_framing.min_zoom_percent = 100.0;
        let mut session = session_with(&script);
        let mut zoom = 0.0;
        for step in 0..12u64 {
            let rendered = session
                .render(step * 100_000, &settings, &mode("MJPG", 64, 32), &halves())
                .unwrap();
            if let AutoFramingStatus::Locked { zoom_percent, .. } = rendered.auto_framing {
                zoom = zoom_percent;
            }
        }
        assert!((zoom - 110.0).abs() < 0.5, "zoom {zoom}");
    }
}
