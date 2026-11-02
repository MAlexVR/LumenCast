//! Face detection for auto framing. The detector is the SeetaFace funnel
//! cascade (via the `rustface` crate): pure Rust, CPU only, and its 1.2 MB
//! model is embedded in the binary. It runs on a small grayscale copy of the
//! frame, because the framing only needs the face position, not its detail.

use lumencast_fx::frame::Frame;
use lumencast_fx::reframe::{FaceBox, FaceDetector};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

/// The model published with SeetaFace (BSD-2-Clause), see `THIRD_PARTY.md`.
const MODEL: &[u8] = include_bytes!("../../../assets/models/seeta_fd_frontal_v1.0.bin");

/// The smallest face (and so the smallest analysed frame) the detector accepts.
const MIN_FACE_SIZE: u32 = 20;

/// The analysed copy is at most this wide.
const ANALYSIS_WIDTH: u32 = 256;

#[derive(Debug, thiserror::Error)]
pub enum VisionError {
    #[error("the embedded face model could not be loaded: {0}")]
    Model(String),
}

/// A grayscale copy of a frame, shrunk by a whole factor.
#[derive(Debug, PartialEq, Eq)]
pub struct Analysis {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    /// How many original pixels one analysed pixel covers, per axis.
    pub scale: u32,
}

/// Shrinks by the smallest whole factor that makes the width at most
/// `max_width`, averaging each block, and converts to luma.
pub fn analysis_copy(frame: &Frame, max_width: u32) -> Analysis {
    let scale = frame.width().div_ceil(max_width.max(1)).max(1);
    let (width, height) = (
        (frame.width() / scale).max(1),
        (frame.height() / scale).max(1),
    );
    let source = frame.data();
    let stride = frame.width() as usize * 3;
    let block = scale as usize;
    let mut pixels = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let mut sum = 0u32;
            for row in y * block..(y * block + block).min(frame.height() as usize) {
                for column in x * block..(x * block + block).min(frame.width() as usize) {
                    let at = row * stride + column * 3;
                    let (r, g, b) = (
                        u32::from(source[at]),
                        u32::from(source[at + 1]),
                        u32::from(source[at + 2]),
                    );
                    sum += (77 * r + 150 * g + 29 * b) >> 8;
                }
            }
            pixels.push((sum / (scale * scale)) as u8);
        }
    }
    Analysis {
        width,
        height,
        pixels,
        scale,
    }
}

/// A detection in analysed pixels, `(x, y, width, height)`, back in frame pixels.
pub fn face_box_from(rect: (i32, i32, u32, u32), scale: u32) -> FaceBox {
    let (x, y, width, height) = rect;
    let scale = scale as f32;
    FaceBox {
        center_x: (x as f32 + width as f32 / 2.0) * scale,
        center_y: (y as f32 + height as f32 / 2.0) * scale,
        height: height as f32 * scale,
    }
}

pub struct SeetaDetector {
    detector: Box<dyn rustface::Detector>,
}

impl SeetaDetector {
    pub fn new() -> Result<Self, VisionError> {
        let model =
            rustface::read_model(MODEL).map_err(|error| VisionError::Model(error.to_string()))?;
        let mut detector = rustface::create_detector_with_model(model);
        detector.set_min_face_size(MIN_FACE_SIZE);
        detector.set_score_thresh(2.0);
        detector.set_pyramid_scale_factor(0.8);
        detector.set_slide_window_step(4, 4);
        Ok(Self { detector })
    }
}

impl FaceDetector for SeetaDetector {
    fn detect(&mut self, frame: &Frame) -> Option<FaceBox> {
        let copy = analysis_copy(frame, ANALYSIS_WIDTH);
        if copy.width < MIN_FACE_SIZE || copy.height < MIN_FACE_SIZE {
            return None;
        }
        let image = rustface::ImageData::new(&copy.pixels, copy.width, copy.height);
        // The person at the camera is the biggest face in the frame.
        let face = self
            .detector
            .detect(&image)
            .into_iter()
            .max_by_key(|face| u64::from(face.bbox().width()) * u64::from(face.bbox().height()))?;
        let bbox = face.bbox();
        Some(face_box_from(
            (bbox.x(), bbox.y(), bbox.width(), bbox.height()),
            copy.scale,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_analysis_copy_is_shrunk_by_the_smallest_whole_factor() {
        let big = analysis_copy(&Frame::from_fn(1280, 720, |_, _| [100; 3]), 320);
        assert_eq!((big.width, big.height, big.scale), (320, 180, 4));
        let odd = analysis_copy(&Frame::from_fn(641, 481, |_, _| [100; 3]), 320);
        assert_eq!(odd.scale, 3);
        assert_eq!((odd.width, odd.height), (641 / 3, 481 / 3));
        let small = analysis_copy(&Frame::from_fn(160, 120, |_, _| [100; 3]), 320);
        assert_eq!((small.width, small.height, small.scale), (160, 120, 1));
    }

    #[test]
    fn the_analysis_copy_is_luma_averaged_over_each_block() {
        let grey = analysis_copy(&Frame::from_fn(8, 8, |_, _| [200; 3]), 4);
        assert!(grey.pixels.iter().all(|value| *value == 200));
        // Green carries most of the luma, blue the least.
        let green = analysis_copy(&Frame::from_fn(4, 4, |_, _| [0, 255, 0]), 4).pixels[0];
        let blue = analysis_copy(&Frame::from_fn(4, 4, |_, _| [0, 0, 255]), 4).pixels[0];
        assert!(green > 140 && blue < 40, "{green} {blue}");
        // Two black and two white pixels of a 2x2 block average to the middle.
        let mixed = analysis_copy(
            &Frame::from_fn(4, 2, |x, _| if x % 2 == 0 { [0; 3] } else { [255; 3] }),
            2,
        );
        assert!(
            (126..=129).contains(&mixed.pixels[0]),
            "{}",
            mixed.pixels[0]
        );
    }

    #[test]
    fn a_detection_is_converted_back_to_frame_pixels() {
        let face = face_box_from((10, 20, 40, 60), 4);
        assert_eq!(
            face,
            FaceBox {
                center_x: 120.0,
                center_y: 200.0,
                height: 240.0
            }
        );
    }

    #[test]
    fn the_embedded_model_loads() {
        assert!(SeetaDetector::new().is_ok());
    }

    #[test]
    fn frames_without_a_face_find_none() {
        let mut detector = SeetaDetector::new().unwrap();
        assert_eq!(
            detector.detect(&Frame::from_fn(1280, 720, |_, _| [90; 3])),
            None
        );
        let noise = Frame::from_fn(640, 480, |x, y| {
            let v = ((x * 7919 + y * 104_729) % 251) as u8;
            [v; 3]
        });
        assert_eq!(detector.detect(&noise), None);
    }

    #[test]
    fn tiny_frames_do_not_panic() {
        let mut detector = SeetaDetector::new().unwrap();
        assert_eq!(detector.detect(&Frame::from_fn(8, 8, |_, _| [1; 3])), None);
        assert_eq!(detector.detect(&Frame::from_fn(1, 1, |_, _| [1; 3])), None);
    }
}

/// Runs a detector on a thread of its own so a slow detection never holds up
/// the caller. `detect` hands the frame over (skipped when the thread is still
/// busy) and answers with the latest finished result, which is therefore a
/// detection or two behind; at about ten calls a second that is under 200 ms.
pub struct AsyncDetector {
    sender: Option<SyncSender<Frame>>,
    latest: Arc<Mutex<Option<FaceBox>>>,
    handle: Option<JoinHandle<()>>,
}

impl AsyncDetector {
    /// `create` runs on the new thread (some detectors cannot cross threads);
    /// its failure is returned here.
    pub fn spawn(
        create: impl FnOnce() -> Result<Box<dyn FaceDetector>, String> + Send + 'static,
    ) -> Result<Self, String> {
        let (sender, frames) = sync_channel::<Frame>(1);
        let (ready_in, ready) = sync_channel::<Result<(), String>>(1);
        let latest = Arc::new(Mutex::new(None));
        let published = Arc::clone(&latest);
        let handle = std::thread::Builder::new()
            .name("lumencast-faces".into())
            .spawn(move || {
                let mut detector = match create() {
                    Ok(detector) => {
                        let _ = ready_in.send(Ok(()));
                        detector
                    }
                    Err(reason) => {
                        let _ = ready_in.send(Err(reason));
                        return;
                    }
                };
                for frame in frames {
                    let found = detector.detect(&frame);
                    if let Ok(mut latest) = published.lock() {
                        *latest = found;
                    }
                }
            })
            .map_err(|error| error.to_string())?;
        ready
            .recv()
            .map_err(|_| "the face detector thread stopped".to_owned())??;
        Ok(Self {
            sender: Some(sender),
            latest,
            handle: Some(handle),
        })
    }

    /// The SeetaFace detector on its own thread.
    pub fn seeta() -> Result<Self, String> {
        Self::spawn(|| {
            SeetaDetector::new()
                .map(|detector| Box::new(detector) as Box<dyn FaceDetector>)
                .map_err(|error| error.to_string())
        })
    }
}

impl FaceDetector for AsyncDetector {
    fn detect(&mut self, frame: &Frame) -> Option<FaceBox> {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(frame.clone());
        }
        self.latest.lock().ok().and_then(|latest| *latest)
    }
}

impl Drop for AsyncDetector {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(test)]
mod async_tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct Slow;

    impl FaceDetector for Slow {
        fn detect(&mut self, frame: &Frame) -> Option<FaceBox> {
            std::thread::sleep(Duration::from_millis(60));
            Some(FaceBox {
                center_x: frame.width() as f32 / 2.0,
                center_y: 5.0,
                height: 10.0,
            })
        }
    }

    #[test]
    fn a_slow_detector_never_blocks_the_caller_and_its_answer_arrives_later() {
        let mut detector =
            AsyncDetector::spawn(|| Ok(Box::new(Slow) as Box<dyn FaceDetector>)).unwrap();
        let frame = Frame::from_fn(40, 20, |_, _| [0; 3]);
        let start = Instant::now();
        assert!(
            detector.detect(&frame).is_none(),
            "nothing has finished yet"
        );
        assert!(
            start.elapsed() < Duration::from_millis(30),
            "the call must not wait"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut found = None;
        while found.is_none() {
            found = detector.detect(&frame);
            assert!(Instant::now() < deadline, "no answer arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(found.unwrap().center_x, 20.0);
    }

    #[test]
    fn a_detector_that_cannot_start_reports_why() {
        let error = AsyncDetector::spawn(|| Err("no model".to_owned()))
            .err()
            .unwrap();
        assert_eq!(error, "no model");
    }
}
