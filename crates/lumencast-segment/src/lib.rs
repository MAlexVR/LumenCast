//! Person segmentation: which pixels are the person and which are background.
//! The model is MediaPipe's selfie segmenter in ONNX form (Apache-2.0, 462 KB,
//! embedded), run on the CPU with `tract`. The matte is computed on a 256x256
//! copy of the frame and smoothed over time so its edges do not flicker.

use lumencast_fx::frame::Frame;
use thiserror::Error;
use tract_onnx::prelude::*;

const MODEL: &[u8] = include_bytes!("../assets/selfie_segmentation_landscape_v1.onnx");
/// Size of the picture the embedded model sees.
pub const WIDTH: usize = 256;
pub const HEIGHT: usize = 144;
/// Share of the previous matte kept at each new one: higher is steadier but lags.
const KEEP_PREVIOUS: f32 = 0.35;

#[derive(Debug, Error)]
pub enum SegmentError {
    #[error("the segmentation model could not be loaded: {0}")]
    Load(String),
    #[error("the segmentation model failed: {0}")]
    Run(String),
}

type Plan = std::sync::Arc<TypedRunnableModel>;

pub use lumencast_fx::matte::Matte;

pub struct SelfieSegmenter {
    plan: Plan,
    width: usize,
    height: usize,
    /// Inputs scaled to -1..1 instead of 0..1.
    signed: bool,
    previous: Option<Vec<f32>>,
}

impl SelfieSegmenter {
    /// The embedded MediaPipe selfie model, the 16:9 (landscape) variant: it
    /// sees webcam frames without squashing them, which gave cleaner edges and
    /// ran in 6 ms against 11 ms for the square variant on the same frame.
    pub fn new() -> Result<Self, SegmentError> {
        Self::with_model(MODEL, WIDTH, HEIGHT, false)
    }

    /// Any ONNX matting model that takes `[1, 3, height, width]` and returns a
    /// `[1, 1, height, width]` matte in 0..1. `signed` scales the input to -1..1.
    pub fn with_model(
        bytes: &[u8],
        width: usize,
        height: usize,
        signed: bool,
    ) -> Result<Self, SegmentError> {
        let load = |error: TractError| SegmentError::Load(format!("{error:#}"));
        let plan = tract_onnx::onnx()
            .with_ignore_value_info(true)
            .model_for_read(&mut std::io::Cursor::new(bytes))
            .map_err(load)?
            .with_input_fact(0, f32::fact([1, 3, height, width]).into())
            .map_err(load)?
            .into_optimized()
            .map_err(load)?
            .into_runnable()
            .map_err(load)?;
        Ok(Self {
            plan,
            width,
            height,
            signed,
            previous: None,
        })
    }

    /// Forgets the temporal state, for example when the scene changes.
    pub fn reset(&mut self) {
        self.previous = None;
    }

    pub fn matte(&mut self, frame: &Frame) -> Result<Matte, SegmentError> {
        let run = |error: TractError| SegmentError::Run(error.to_string());
        let input = Tensor::from_shape(
            &[1, 3, self.height, self.width],
            &planes(frame, self.width, self.height, self.signed),
        )
        .map_err(run)?;
        let outputs = self.plan.run(tvec!(input.into())).map_err(run)?;
        let alpha = outputs[0].to_plain_array_view::<f32>().map_err(run)?;
        let mut mixed: Vec<f32> = alpha.iter().map(|v| v.clamp(0.0, 1.0)).collect();
        if let Some(previous) = self.previous.as_ref().filter(|p| p.len() == mixed.len()) {
            smooth(&mut mixed, previous);
        }
        let matte = Matte::new(
            self.width,
            self.height,
            mixed.iter().map(|v| (v * 255.0).round() as u8).collect(),
        )
        .ok_or_else(|| SegmentError::Run("the model returned a matte of the wrong size".into()))?;
        self.previous = Some(mixed);
        Ok(matte)
    }
}

/// Blends the new matte with the previous one, in place.
fn smooth(new: &mut [f32], previous: &[f32]) {
    for (new, old) in new.iter_mut().zip(previous) {
        *new = *new * (1.0 - KEEP_PREVIOUS) + old * KEEP_PREVIOUS;
    }
}

/// Channel planes of the frame shrunk to `width` x `height` by averaging the
/// source pixels each model pixel covers, scaled to 0..1 (or -1..1).
fn planes(frame: &Frame, out_width: usize, out_height: usize, signed: bool) -> Vec<f32> {
    let (width, height) = (frame.width() as usize, frame.height() as usize);
    let data = frame.data();
    let plane = out_width * out_height;
    let mut out = vec![0.0_f32; 3 * plane];
    for y in 0..out_height {
        let y0 = y * height / out_height;
        let y1 = ((y + 1) * height / out_height).max(y0 + 1);
        for x in 0..out_width {
            let x0 = x * width / out_width;
            let x1 = ((x + 1) * width / out_width).max(x0 + 1);
            let mut sum = [0u32; 3];
            // At most four samples per axis keeps the cost flat for big frames.
            let step_y = ((y1 - y0) / 4).max(1);
            let step_x = ((x1 - x0) / 4).max(1);
            let mut count = 0u32;
            for sy in (y0..y1).step_by(step_y) {
                for sx in (x0..x1).step_by(step_x) {
                    let at = (sy * width + sx) * 3;
                    for c in 0..3 {
                        sum[c] += u32::from(data[at + c]);
                    }
                    count += 1;
                }
            }
            for c in 0..3 {
                let unit = sum[c] as f32 / count as f32 / 255.0;
                out[c * plane + y * out_width + x] = if signed { unit * 2.0 - 1.0 } else { unit };
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(value: u8) -> Frame {
        Frame::from_fn(640, 360, |_, _| [value; 3])
    }

    #[test]
    fn the_model_loads_and_gives_a_full_matte() {
        let mut segmenter = SelfieSegmenter::new().unwrap();
        let matte = segmenter.matte(&flat(128)).unwrap();
        assert_eq!(matte.alpha().len(), WIDTH * HEIGHT);
    }

    #[test]
    fn smoothing_blends_the_previous_matte_in() {
        let mut new = vec![1.0, 0.0];
        smooth(&mut new, &[0.0, 1.0]);
        assert!((new[0] - (1.0 - KEEP_PREVIOUS)).abs() < 1e-6);
        assert!((new[1] - KEEP_PREVIOUS).abs() < 1e-6);
    }

    #[test]
    fn a_second_frame_runs_with_history_and_a_reset_forgets_it() {
        let mut segmenter = SelfieSegmenter::new().unwrap();
        segmenter.matte(&flat(30)).unwrap();
        assert!(segmenter.previous.is_some());
        segmenter.matte(&flat(220)).unwrap();
        segmenter.reset();
        assert!(segmenter.previous.is_none());
    }
}
