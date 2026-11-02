//! Output and preview pipelines. The preview starts from the output frame, so
//! a stage that exists only for the preview (the mirror) never reaches the
//! transmitted video.

use crate::frame::Frame;
use crate::stages::Stage;

#[derive(Default)]
pub struct Pipeline {
    output: Vec<Box<dyn Stage>>,
    preview_only: Vec<Box<dyn Stage>>,
}

pub struct Processed {
    pub output: Frame,
    pub preview: Frame,
}

impl Pipeline {
    pub fn new() -> Self {
        Self::default()
    }

    /// A stage applied to both the transmitted output and the preview.
    pub fn with_output_stage(mut self, stage: impl Stage + 'static) -> Self {
        self.output.push(Box::new(stage));
        self
    }

    /// A stage applied after the output is fixed, for the preview only.
    pub fn with_preview_stage(mut self, stage: impl Stage + 'static) -> Self {
        self.preview_only.push(Box::new(stage));
        self
    }

    pub fn run(&self, frame: Frame) -> Processed {
        let output = self
            .output
            .iter()
            .fold(frame, |frame, stage| stage.apply(frame));
        let preview = self
            .preview_only
            .iter()
            .fold(output.clone(), |frame, stage| stage.apply(frame));
        Processed { output, preview }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::{Pan, Strength, Zoom};
    use crate::stages::{ManualFraming, Mirror, Vignette};

    fn frame() -> Frame {
        Frame::from_fn(8, 4, |x, y| [x as u8 * 20, y as u8 * 20, 5])
    }

    #[test]
    fn an_empty_pipeline_passes_the_frame_through_to_both() {
        let processed = Pipeline::new().run(frame());
        assert_eq!(processed.output, frame());
        assert_eq!(processed.preview, frame());
    }

    #[test]
    fn the_mirror_reaches_the_preview_but_never_the_output() {
        let processed = Pipeline::new().with_preview_stage(Mirror).run(frame());
        assert_eq!(processed.output, frame());
        assert_eq!(processed.preview, Mirror.apply(frame()));
        assert_ne!(processed.preview, processed.output);
    }

    #[test]
    fn output_stages_apply_to_both_in_order_before_preview_only_stages() {
        let framing = ManualFraming {
            zoom: Zoom::new(2.0).unwrap(),
            pan_x: Pan::new(0.0).unwrap(),
            pan_y: Pan::new(0.0).unwrap(),
        };
        let vignette = Vignette {
            strength: Strength::new(0.5).unwrap(),
        };
        let processed = Pipeline::new()
            .with_output_stage(framing)
            .with_output_stage(vignette)
            .with_preview_stage(Mirror)
            .run(frame());
        let expected_output = vignette.apply(framing.apply(frame()));
        assert_eq!(processed.output, expected_output);
        assert_eq!(processed.preview, Mirror.apply(expected_output));
    }
}
