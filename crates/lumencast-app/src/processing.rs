//! User-facing processing settings and the pipeline they describe. Field
//! names and ranges follow the legacy Blucast settings (`zoom_factor` in
//! percent, `pan_x`, `pan_y`, `mirror_preview`).

use crate::error::AppError;
use lumencast_fx::error::FxError;
use lumencast_fx::keylight::KeyLight;
use lumencast_fx::lightprobe::LightProbe;
use lumencast_fx::params::{Pan, Strength, Zoom};
use lumencast_fx::pipeline::Pipeline;
use lumencast_fx::reframe::AutoFraming;
use lumencast_fx::reframe::FaceBox;
use lumencast_fx::stages::{crop_window, FaceVignette, ManualFraming, Mirror, Vignette};
use serde::{Deserialize, Serialize};

/// Follows the face by panning at a fixed zoom, as NVIDIA Broadcast's auto
/// frame does with its zoom amount. While it has a face it replaces the manual
/// zoom and pan; before the first face it leaves them alone.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AutoFramingSettings {
    pub enabled: bool,
    /// 100 to 200: the zoom kept while following (it never changes to chase a face).
    pub min_zoom_percent: f32,
    /// 0 (slow and smooth) to 100 (fast).
    pub speed: u8,
}

impl Default for AutoFramingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            min_zoom_percent: 130.0,
            speed: 35,
        }
    }
}

/// What happens to everything behind the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundMode {
    #[default]
    None,
    Blur,
    /// A white background, as Blucast's "Quitar" did.
    Remove,
    Replace,
}

/// Which model finds the person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackgroundEngine {
    /// NVIDIA's own model, run on the GPU in the user's container (as Blucast
    /// did); the built-in one takes over while it is not available.
    #[default]
    Nvidia,
    /// The small model embedded in the program, on the CPU.
    Builtin,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct BackgroundSettings {
    pub engine: BackgroundEngine,
    pub mode: BackgroundMode,
    /// 0 to 100.
    pub blur_strength: u8,
    /// PNG or JPEG used by `Replace`.
    pub image_path: Option<String>,
}

impl Default for BackgroundSettings {
    fn default() -> Self {
        Self {
            engine: BackgroundEngine::Nvidia,
            mode: BackgroundMode::None,
            blur_strength: 50,
            image_path: None,
        }
    }
}

impl BackgroundSettings {
    fn validate(&self) -> Result<(), AppError> {
        if self.blur_strength > 100 {
            return Err(AppError::InvalidSetting(format!(
                "the background blur strength must be between 0 and 100, got {}",
                self.blur_strength
            )));
        }
        Ok(())
    }
}

/// Cleans low-light noise (see `lumencast_fx::denoise`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DenoiseSettings {
    pub enabled: bool,
    /// 0 to 100.
    pub strength: u8,
}

impl Default for DenoiseSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            strength: 60,
        }
    }
}

impl DenoiseSettings {
    fn validate(&self) -> Result<(), AppError> {
        if self.strength > 100 {
            return Err(AppError::InvalidSetting(format!(
                "the noise reduction strength must be between 0 and 100, got {}",
                self.strength
            )));
        }
        Ok(())
    }

    /// The strength to use, or `None` while the effect is off.
    pub fn level(&self) -> Option<Strength> {
        if !self.enabled || self.strength == 0 {
            return None;
        }
        Strength::new(f32::from(self.strength) / 100.0).ok()
    }
}

/// Sends the processed video to other programs as a virtual camera.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct VirtualCameraSettings {
    pub enabled: bool,
    /// Scale the video that goes out up to this height with a Lanczos filter
    /// (0 leaves it as it is); never scales down. NVIDIA's `video_super_resolution`
    /// sets an output size too, with a neural model.
    pub upscale_height: u32,
}

impl VirtualCameraSettings {
    fn validate(&self) -> Result<(), AppError> {
        if self.upscale_height != 0 && !(240..=2160).contains(&self.upscale_height) {
            return Err(AppError::InvalidSetting(format!(
                "the virtual camera output height must be 0 or between 240 and 2160, got {}",
                self.upscale_height
            )));
        }
        Ok(())
    }

    /// The size a `width` x `height` frame is scaled to, or `None` when it stays.
    pub fn upscaled_size(&self, width: u32, height: u32) -> Option<(u32, u32)> {
        if self.upscale_height == 0 || self.upscale_height <= height || height == 0 {
            return None;
        }
        let target_width =
            (u64::from(width) * u64::from(self.upscale_height) / u64::from(height)) as u32;
        // Video wants even sizes.
        Some((target_width & !1, self.upscale_height & !1))
    }
}

/// Where a face lands, as fractions of the framed picture, once the crop for
/// `zoom` and the pans has been applied to a `width` x `height` frame.
fn vignette_centre(
    face: FaceBox,
    width: u32,
    height: u32,
    zoom: Zoom,
    pan_x: Pan,
    pan_y: Pan,
) -> (f32, f32) {
    let window = crop_window(width, height, zoom, pan_x, pan_y);
    let along = |position: f32, start: u32, length: u32| {
        ((position - start as f32) / length.max(1) as f32).clamp(0.0, 1.0)
    };
    (
        along(face.center_x, window.x, window.width),
        along(face.center_y, window.y, window.height),
    )
}

/// A soft light aimed at the face (see `lumencast_fx::keylight`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct KeyLightSettings {
    pub enabled: bool,
    /// 0 to 100.
    pub intensity: u8,
    /// 0 (cool) to 100 (warm).
    pub warmth: u8,
    /// A picture of a light (a studio, a window, a lamp): the key light comes
    /// from where the picture is brightest, with its colour. NVIDIA's version
    /// of this is an HDRI environment map.
    pub light_picture: Option<String>,
}

impl Default for KeyLightSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: 40,
            warmth: 60,
            light_picture: None,
        }
    }
}

impl KeyLightSettings {
    fn validate(&self) -> Result<(), AppError> {
        for (name, value) in [("intensity", self.intensity), ("warmth", self.warmth)] {
            if value > 100 {
                return Err(AppError::InvalidSetting(format!(
                    "the key light {name} must be between 0 and 100, got {value}"
                )));
            }
        }
        if let Some(path) = &self.light_picture {
            if path.is_empty() || path.contains(['\n', '\0']) {
                return Err(AppError::InvalidSetting(
                    "the key light picture path is not valid".into(),
                ));
            }
        }
        Ok(())
    }

    /// The stage for a face, or `None` when the light is off. `probe` is what
    /// was read from the light picture, when there is one.
    pub fn stage(&self, face: FaceBox, probe: Option<LightProbe>) -> Option<KeyLight> {
        if !self.enabled || self.intensity == 0 {
            return None;
        }
        Some(KeyLight {
            face,
            intensity: Strength::new(f32::from(self.intensity) / 100.0).ok()?,
            warmth: Strength::new(f32::from(self.warmth) / 100.0).ok()?,
            probe,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessingSettings {
    /// 100 to 200; 100 means no zoom.
    pub zoom_percent: f32,
    /// -1 to 1.
    pub pan_x: f32,
    /// -1 to 1.
    pub pan_y: f32,
    /// Flips only the local preview, never the transmitted video.
    pub mirror_preview: bool,
    /// 0 to 1; 0 means off.
    pub vignette: f32,
    /// Centre the vignette on the face instead of the middle of the picture
    /// (NVIDIA's `vignette.faceTracking`).
    pub vignette_follows_face: bool,
    pub auto_framing: AutoFramingSettings,
    pub key_light: KeyLightSettings,
    pub background: BackgroundSettings,
    pub denoise: DenoiseSettings,
    pub virtual_camera: VirtualCameraSettings,
}

impl Default for ProcessingSettings {
    fn default() -> Self {
        Self {
            zoom_percent: 100.0,
            pan_x: 0.0,
            pan_y: 0.0,
            mirror_preview: false,
            vignette: 0.0,
            vignette_follows_face: false,
            auto_framing: AutoFramingSettings::default(),
            key_light: KeyLightSettings::default(),
            background: BackgroundSettings::default(),
            denoise: DenoiseSettings::default(),
            virtual_camera: VirtualCameraSettings::default(),
        }
    }
}

impl ProcessingSettings {
    pub fn validate(&self) -> Result<(), AppError> {
        self.parts().map(drop)
    }

    /// True when processing would change nothing, so frames can pass through.
    pub fn is_identity(&self) -> bool {
        self.zoom_percent <= 100.0
            && !self.mirror_preview
            && self.vignette == 0.0
            && !self.auto_framing.enabled
            && !self.key_light.enabled
            && self.background.mode == BackgroundMode::None
            && !self.denoise.enabled
            && !self.virtual_camera.enabled
    }

    fn validate_auto_framing(&self) -> Result<(), AppError> {
        let auto = &self.auto_framing;
        if !(100.0..=200.0).contains(&auto.min_zoom_percent) {
            return Err(AppError::InvalidSetting(format!(
                "the auto framing minimum zoom must be between 100 and 200, got {}",
                auto.min_zoom_percent
            )));
        }
        if auto.speed > 100 {
            return Err(AppError::InvalidSetting(format!(
                "the auto framing speed must be between 0 and 100, got {}",
                auto.speed
            )));
        }
        Ok(())
    }

    pub fn pipeline(&self) -> Result<Pipeline, AppError> {
        self.pipeline_with(None, None)
    }

    /// The pipeline, with the auto framing (when it has a face) replacing the
    /// manual zoom and pan.
    ///
    /// `face` is where the face was found (its centre and the size of the frame
    /// it was found in); a vignette that follows the face is centred on it, as
    /// it lands after the framing.
    pub fn pipeline_with(
        &self,
        auto: Option<AutoFraming>,
        face: Option<(FaceBox, u32, u32)>,
    ) -> Result<Pipeline, AppError> {
        let (mut zoom, mut pan_x, mut pan_y, vignette) = self.parts()?;
        if let Some(auto) = auto {
            (zoom, pan_x, pan_y) = auto.as_params();
        }
        let mut pipeline = Pipeline::new();
        if zoom.get() > 1.0 {
            pipeline = pipeline.with_output_stage(ManualFraming { zoom, pan_x, pan_y });
        }
        if vignette.get() > 0.0 {
            pipeline = match face.filter(|_| self.vignette_follows_face) {
                Some((face, width, height)) => {
                    let centre = vignette_centre(face, width, height, zoom, pan_x, pan_y);
                    pipeline.with_output_stage(FaceVignette {
                        strength: vignette,
                        centre,
                    })
                }
                None => pipeline.with_output_stage(Vignette { strength: vignette }),
            };
        }
        if self.mirror_preview {
            pipeline = pipeline.with_preview_stage(Mirror);
        }
        Ok(pipeline)
    }

    fn parts(&self) -> Result<(Zoom, Pan, Pan, Strength), AppError> {
        self.validate_auto_framing()?;
        self.key_light.validate()?;
        self.background.validate()?;
        self.denoise.validate()?;
        self.virtual_camera.validate()?;
        let invalid = |error: FxError| AppError::InvalidSetting(error.to_string());
        Ok((
            Zoom::new(self.zoom_percent / 100.0).map_err(invalid)?,
            Pan::new(self.pan_x).map_err(invalid)?,
            Pan::new(self.pan_y).map_err(invalid)?,
            Strength::new(self.vignette).map_err(invalid)?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lumencast_fx::frame::Frame;

    fn settings(
        zoom: f32,
        pan_x: f32,
        pan_y: f32,
        mirror: bool,
        vignette: f32,
    ) -> ProcessingSettings {
        ProcessingSettings {
            zoom_percent: zoom,
            pan_x,
            pan_y,
            mirror_preview: mirror,
            vignette,
            ..ProcessingSettings::default()
        }
    }

    #[test]
    fn defaults_change_nothing() {
        assert!(ProcessingSettings::default().is_identity());
        assert!(ProcessingSettings::default().validate().is_ok());
    }

    #[test]
    fn any_active_effect_makes_it_not_the_identity_but_pan_alone_does_not() {
        assert!(!settings(100.0, 0.0, 0.0, true, 0.0).is_identity());
        assert!(!settings(150.0, 0.0, 0.0, false, 0.0).is_identity());
        assert!(!settings(100.0, 0.0, 0.0, false, 0.2).is_identity());
        // Panning does nothing without a zoom: there is no margin to move in.
        assert!(settings(100.0, 0.9, -0.9, false, 0.0).is_identity());
    }

    #[test]
    fn values_outside_the_legacy_ranges_are_rejected_as_invalid_settings() {
        for bad in [
            settings(99.9, 0.0, 0.0, false, 0.0),
            settings(200.1, 0.0, 0.0, false, 0.0),
            settings(150.0, 1.01, 0.0, false, 0.0),
            settings(150.0, 0.0, -1.01, false, 0.0),
            settings(150.0, 0.0, 0.0, false, 1.1),
            settings(f32::NAN, 0.0, 0.0, false, 0.0),
            settings(150.0, f32::INFINITY, 0.0, false, 0.0),
        ] {
            let error = bad.validate().unwrap_err();
            assert_eq!(error.code(), "invalid_setting", "{bad:?}");
        }
        assert!(settings(100.0, -1.0, 1.0, true, 1.0).validate().is_ok());
        assert!(settings(200.0, 1.0, -1.0, false, 0.0).validate().is_ok());
    }

    #[test]
    fn the_pipeline_mirrors_only_the_preview_and_zooms_both() {
        let frame = Frame::from_fn(
            40,
            4,
            |x, _| {
                if x < 20 {
                    [0, 0, 0]
                } else {
                    [255, 255, 255]
                }
            },
        );
        let mirrored = settings(100.0, 0.0, 0.0, true, 0.0)
            .pipeline()
            .unwrap()
            .run(frame.clone());
        assert_eq!(mirrored.output, frame);
        assert_eq!(mirrored.preview.pixel(2, 0), [255, 255, 255]);
        assert_eq!(mirrored.preview.pixel(37, 0), [0, 0, 0]);

        // Zoom 2 panned fully right shows only the white half, in output and preview.
        let zoomed = settings(200.0, 1.0, 0.0, false, 0.0)
            .pipeline()
            .unwrap()
            .run(frame);
        assert!((0..40).all(|x| zoomed.output.pixel(x, 1)[0] >= 250));
        assert_eq!(zoomed.output, zoomed.preview);
    }

    #[test]
    fn auto_framing_is_off_by_default_and_counts_as_an_effect_when_on() {
        let defaults = ProcessingSettings::default().auto_framing;
        assert_eq!(
            (defaults.enabled, defaults.min_zoom_percent, defaults.speed),
            (false, 130.0, 35)
        );
        let mut on = ProcessingSettings::default();
        on.auto_framing.enabled = true;
        assert!(!on.is_identity());
        assert!(on.validate().is_ok());
    }

    #[test]
    fn auto_framing_values_outside_their_ranges_are_invalid_settings() {
        for bad in [(99.0, 35), (201.0, 35), (150.0, 101), (f32::NAN, 35)] {
            let mut settings = ProcessingSettings::default();
            settings.auto_framing.min_zoom_percent = bad.0;
            settings.auto_framing.speed = bad.1;
            assert_eq!(
                settings.validate().unwrap_err().code(),
                "invalid_setting",
                "{bad:?}"
            );
        }
    }

    #[test]
    fn auto_framing_deserializes_from_camel_case_with_defaults() {
        let parsed: ProcessingSettings =
            serde_json::from_str(r#"{"autoFraming":{"enabled":true,"speed":80}}"#).unwrap();
        assert!(parsed.auto_framing.enabled);
        assert_eq!(parsed.auto_framing.speed, 80);
        assert_eq!(parsed.auto_framing.min_zoom_percent, 130.0);
        assert_eq!(parsed.zoom_percent, 100.0);
    }

    #[test]
    fn settings_deserialize_from_camel_case_with_defaults_for_missing_fields() {
        let parsed: ProcessingSettings =
            serde_json::from_str(r#"{"zoomPercent":150,"panX":0.5,"mirrorPreview":true}"#).unwrap();
        assert_eq!(parsed, settings(150.0, 0.5, 0.0, true, 0.0));
        let json = serde_json::to_value(parsed).unwrap();
        assert_eq!(json["zoomPercent"], 150.0);
        assert_eq!(json["vignette"], 0.0);
    }

    #[test]
    fn the_key_light_has_defaults_and_validates_its_range() {
        let defaults = ProcessingSettings::default();
        assert!(!defaults.key_light.enabled);
        let mut bad = defaults;
        bad.key_light.warmth = 101;
        assert!(matches!(bad.validate(), Err(AppError::InvalidSetting(_))));
        let parsed: ProcessingSettings =
            serde_json::from_str(r#"{"keyLight":{"enabled":true,"intensity":70}}"#).unwrap();
        assert!(parsed.key_light.enabled);
        assert_eq!(
            (parsed.key_light.intensity, parsed.key_light.warmth),
            (70, 60)
        );
    }

    #[test]
    fn an_enabled_key_light_means_frames_cannot_pass_through_untouched() {
        let mut settings = ProcessingSettings::default();
        assert!(settings.is_identity());
        settings.key_light.enabled = true;
        assert!(!settings.is_identity());
    }

    #[test]
    fn the_key_light_stage_exists_only_while_on_and_not_at_zero() {
        let face = FaceBox {
            center_x: 10.0,
            center_y: 10.0,
            height: 5.0,
        };
        let mut light = KeyLightSettings::default();
        assert!(light.stage(face, None).is_none());
        light.enabled = true;
        assert!(light.stage(face, None).is_some());
        light.intensity = 0;
        assert!(light.stage(face, None).is_none());
    }

    #[test]
    fn the_noise_reduction_validates_and_only_has_a_level_while_on() {
        let mut settings = ProcessingSettings::default();
        assert!(settings.denoise.level().is_none());
        settings.denoise.enabled = true;
        assert!(settings.denoise.level().is_some());
        assert!(!settings.is_identity());
        settings.denoise.strength = 101;
        assert!(matches!(
            settings.validate(),
            Err(AppError::InvalidSetting(_))
        ));
        settings.denoise.strength = 0;
        assert!(settings.denoise.level().is_none());
    }

    #[test]
    fn a_vignette_that_follows_the_face_is_centred_on_it_after_the_crop() {
        let face = FaceBox {
            center_x: 320.0,
            center_y: 180.0,
            height: 100.0,
        };
        let mut settings = ProcessingSettings {
            vignette: 1.0,
            vignette_follows_face: true,
            ..ProcessingSettings::default()
        };
        let frame = || Frame::from_fn(640, 360, |_, _| [200; 3]);
        // The face is on the left half: the vignette must leave it brighter than the right side.
        let at_left = FaceBox {
            center_x: 100.0,
            ..face
        };
        let out = settings
            .pipeline_with(None, Some((at_left, 640, 360)))
            .unwrap()
            .run(frame())
            .output;
        assert!(out.pixel(100, 180)[0] > out.pixel(540, 180)[0]);
        // Without a face, or without the option, the plain centred vignette applies.
        let plain = settings
            .pipeline_with(None, None)
            .unwrap()
            .run(frame())
            .output;
        assert_eq!(plain.pixel(100, 180), plain.pixel(540, 180));
        settings.vignette_follows_face = false;
        let ignored = settings
            .pipeline_with(None, Some((at_left, 640, 360)))
            .unwrap()
            .run(frame())
            .output;
        assert_eq!(ignored, plain);
    }

    #[test]
    fn the_output_is_only_ever_scaled_up_and_to_even_sizes() {
        let mut camera = VirtualCameraSettings {
            enabled: true,
            upscale_height: 1080,
        };
        assert_eq!(camera.upscaled_size(1280, 720), Some((1920, 1080)));
        assert_eq!(camera.upscaled_size(640, 480), Some((1440, 1080)));
        assert_eq!(camera.upscaled_size(1920, 1080), None, "already that tall");
        assert_eq!(camera.upscaled_size(3840, 2160), None, "never scaled down");
        camera.upscale_height = 0;
        assert_eq!(camera.upscaled_size(1280, 720), None);
        camera.upscale_height = 1001;
        assert_eq!(camera.upscaled_size(1280, 720).unwrap().1 % 2, 0);
    }

    #[test]
    fn an_absurd_output_height_is_rejected() {
        let mut settings = ProcessingSettings::default();
        settings.virtual_camera.upscale_height = 100_000;
        assert!(matches!(
            settings.validate(),
            Err(AppError::InvalidSetting(_))
        ));
        settings.virtual_camera.upscale_height = 1080;
        assert!(settings.validate().is_ok());
    }
}
