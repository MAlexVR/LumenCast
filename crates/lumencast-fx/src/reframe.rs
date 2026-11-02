//! Auto framing: turns the position of a face into a zoom and pan, smoothed
//! over time. The maths follow the legacy Blucast `AutoReframer` so the
//! behaviour users know is kept; only the time handling is new.

use crate::params::{Pan, Zoom};

/// Keep the face at about this fraction of the frame height.
/// The legacy server detected about 15 times per second; smoothing is defined per such step.
/// How far the face may move, as a share of the frame, before the framing
/// follows (the detector's box trembles by a few pixels even when you sit still).
/// The share of a new detection that enters the running average.
const DETECTION_SHARE: f32 = 0.4;
const DEAD_ZONE_X: f32 = 0.025;
const DEAD_ZONE_Y: f32 = 0.035;
const REFERENCE_STEP_US: f32 = 66_667.0;
const SMOOTHING_SLOWEST: f32 = 0.95;
const SMOOTHING_FASTEST: f32 = 0.5;

/// A face found in a frame, in pixels of that frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FaceBox {
    pub center_x: f32,
    pub center_y: f32,
    pub height: f32,
}

/// Something that finds the main face in a frame. The model behind it is not
/// this crate's business.
pub trait FaceDetector {
    fn detect(&mut self, frame: &crate::frame::Frame) -> Option<FaceBox>;
}

/// A zoom (1.0 to 2.0) and a pan (-1.0 to 1.0) for `ManualFraming`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AutoFraming {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
}

impl AutoFraming {
    /// The framing as validated stage parameters.
    pub fn as_params(&self) -> (Zoom, Pan, Pan) {
        // Smoothing can leave values a rounding error outside the range.
        (
            Zoom::new(self.zoom.clamp(1.0, 2.0)).unwrap_or_default(),
            Pan::new(self.pan_x.clamp(-1.0, 1.0)).unwrap_or_default(),
            Pan::new(self.pan_y.clamp(-1.0, 1.0)).unwrap_or_default(),
        )
    }
}

/// Time constant of the per-frame easing, in microseconds. Detection runs a few
/// times per second, so the framing it asks for is reached gradually, frame by
/// frame, instead of in steps.
const EASE_TIME_CONSTANT_US: f32 = 120_000.0;

impl AutoFraming {
    /// Moves from `self` towards `target` for `elapsed_us` of capture time. The
    /// result depends on elapsed time, not on how many frames it took.
    pub fn eased_towards(self, target: AutoFraming, elapsed_us: u64) -> AutoFraming {
        let share = 1.0 - (-(elapsed_us as f32) / EASE_TIME_CONSTANT_US).exp();
        let lerp = |from: f32, to: f32| from + (to - from) * share;
        AutoFraming {
            zoom: lerp(self.zoom, target.zoom),
            pan_x: lerp(self.pan_x, target.pan_x),
            pan_y: lerp(self.pan_y, target.pan_y),
        }
    }
}

/// The legacy slider: speed 0 is the slowest (smoothing 0.95), 100 the fastest (0.5).
pub fn smoothing_for_speed(speed: u8) -> f32 {
    let speed = f32::from(speed.min(100)) / 100.0;
    SMOOTHING_SLOWEST - speed * (SMOOTHING_SLOWEST - SMOOTHING_FASTEST)
}

/// Where the framing should end up for this face, before any smoothing.
pub fn target_framing(
    face: FaceBox,
    frame_width: u32,
    frame_height: u32,
    min_zoom: f32,
) -> AutoFraming {
    let (width, height) = (frame_width as f32, frame_height as f32);
    // The zoom is the user's, as in NVIDIA Broadcast's `auto_frame` (a zoom
    // amount plus face tracking): following the face only moves the crop, it
    // never changes how close the picture is.
    let zoom = min_zoom.clamp(1.0, 2.0);
    let pan = |center: f32, length: f32| {
        let crop = length / zoom;
        let margin = (length - crop) / 2.0;
        if margin > 0.5 {
            ((center - crop / 2.0 - margin) / margin).clamp(-1.0, 1.0)
        } else {
            0.0
        }
    };
    AutoFraming {
        zoom,
        pan_x: pan(face.center_x, width),
        pan_y: pan(face.center_y, height),
    }
}

/// Whether the crop at `zoom` is wide enough to bring the face to its centre.
/// Beyond the margin the pan sits at its limit and the face stays off centre.
pub fn within_reach(face: FaceBox, frame_width: u32, frame_height: u32, zoom: f32) -> bool {
    let zoom = zoom.clamp(1.0, 2.0);
    let reaches = |center: f32, length: f32| {
        let margin = (length - length / zoom) / 2.0;
        // Where the crop start would have to be, against the room there is.
        margin <= 0.5 || ((center - length / zoom / 2.0 - margin) / margin).abs() <= 1.0
    };
    reaches(face.center_x, frame_width as f32) && reaches(face.center_y, frame_height as f32)
}

/// Smooths successive faces into a steady framing.
#[derive(Debug, Clone)]
pub struct Reframer {
    min_zoom: f32,
    smoothing: f32,
    current: AutoFraming,
    locked: bool,
    reachable: bool,
    /// The face position the framing is aimed at: it moves only when the face
    /// really moves, not with the detector's noise.
    held: Option<(f32, f32)>,
    /// The detections averaged a little, before the dead zone looks at them.
    averaged: Option<(f32, f32)>,
}

impl Reframer {
    pub fn new(min_zoom: Zoom, speed: u8) -> Self {
        Self {
            min_zoom: min_zoom.get(),
            smoothing: smoothing_for_speed(speed),
            current: AutoFraming {
                zoom: min_zoom.get(),
                pan_x: 0.0,
                pan_y: 0.0,
            },
            locked: false,
            reachable: true,
            held: None,
            averaged: None,
        }
    }

    /// The zoom to keep while following. It applies at once: the pan is what
    /// gets smoothed, never the user's own zoom.
    pub fn set_min_zoom(&mut self, min_zoom: Zoom) {
        self.min_zoom = min_zoom.get();
        self.current.zoom = min_zoom.get();
    }

    pub fn set_speed(&mut self, speed: u8) {
        self.smoothing = smoothing_for_speed(speed);
    }

    /// Whether the last face found can be centred at this zoom.
    pub fn reachable(&self) -> bool {
        self.reachable
    }

    /// The framing as it stands, without feeding a detection; `None` until a
    /// face has been found.
    pub fn current(&self) -> Option<AutoFraming> {
        self.locked.then_some(self.current)
    }

    /// The face the framing aims at: the new one, unless it is within the dead
    /// zone of the held one, in which case the held one stays; a bigger move
    /// drags the held position along by the excess only, so it never jumps.
    fn hold(&mut self, face: FaceBox, frame_width: u32, frame_height: u32) -> FaceBox {
        let (band_x, band_y) = (
            DEAD_ZONE_X * frame_width as f32,
            DEAD_ZONE_Y * frame_height as f32,
        );
        // The box found by the detector trembles most up and down (its height
        // changes from frame to frame), so detections are averaged first.
        let (ax, ay) = match self.averaged {
            None => (face.center_x, face.center_y),
            Some((x, y)) => (
                x + DETECTION_SHARE * (face.center_x - x),
                y + DETECTION_SHARE * (face.center_y - y),
            ),
        };
        self.averaged = Some((ax, ay));
        let face = FaceBox {
            center_x: ax,
            center_y: ay,
            ..face
        };
        let follow = |held: f32, new: f32, band: f32| {
            let delta = new - held;
            if delta.abs() <= band {
                held
            } else {
                held + delta - band * delta.signum()
            }
        };
        let (x, y) = match self.held {
            None => (face.center_x, face.center_y),
            Some((x, y)) => (
                follow(x, face.center_x, band_x),
                follow(y, face.center_y, band_y),
            ),
        };
        self.held = Some((x, y));
        FaceBox {
            center_x: x,
            center_y: y,
            ..face
        }
    }

    /// Feeds one detection result. `elapsed_us` is the capture time since the
    /// previous call; the smoothing is defined per legacy detection step
    /// (about 15 per second) and scaled by this. Returns the framing to use,
    /// or `None` while no face has ever been found, so the caller keeps the
    /// manual framing. A missed detection holds the last framing.
    pub fn observe(
        &mut self,
        face: Option<FaceBox>,
        frame_width: u32,
        frame_height: u32,
        elapsed_us: u64,
    ) -> Option<AutoFraming> {
        if let Some(face) = face {
            let steps = elapsed_us.max(1) as f32 / REFERENCE_STEP_US;
            let zoom = self.min_zoom;
            let face = self.hold(face, frame_width, frame_height);
            let target = target_framing(face, frame_width, frame_height, zoom);
            self.reachable = within_reach(face, frame_width, frame_height, zoom);
            // How much of the old framing survives this step.
            let retained = self.smoothing.powf(steps);
            let blend = |old: f32, new: f32| old * retained + new * (1.0 - retained);
            self.current = AutoFraming {
                zoom: target.zoom,
                pan_x: blend(self.current.pan_x, target.pan_x),
                pan_y: blend(self.current.pan_y, target.pan_y),
            };
            self.locked = true;
        }
        self.locked.then_some(self.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(center_x: f32, center_y: f32, height: f32) -> FaceBox {
        FaceBox {
            center_x,
            center_y,
            height,
        }
    }

    fn zoom(value: f32) -> Zoom {
        Zoom::new(value).unwrap()
    }

    const STEP: u64 = 66_667;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn the_speed_slider_maps_to_the_legacy_smoothing_range() {
        assert!(close(smoothing_for_speed(0), 0.95));
        assert!(close(smoothing_for_speed(100), 0.5));
        assert!(close(smoothing_for_speed(80), 0.59));
        assert!(close(smoothing_for_speed(35), 0.7925));
        assert!(
            close(smoothing_for_speed(200), 0.5),
            "values above 100 are clamped"
        );
    }

    #[test]
    fn the_zoom_is_the_one_asked_for_whatever_the_size_of_the_face() {
        for height in [50.0, 140.0, 500.0] {
            let target = target_framing(face(640.0, 360.0, height), 1280, 720, 1.3);
            assert!(close(target.zoom, 1.3), "{target:?}");
        }
        assert!(close(
            target_framing(face(640.0, 360.0, 140.0), 1280, 720, 2.5).zoom,
            2.0
        ));
        assert!(close(
            target_framing(face(640.0, 360.0, 140.0), 1280, 720, 0.5).zoom,
            1.0
        ));
    }

    #[test]
    fn the_pan_follows_the_face_inside_the_margin_the_crop_leaves() {
        // Zoom 1.8: crop 711.1 wide, margin 284.4. A face at x = 700 is 60 px
        // right of the crop centre, so the pan is 60 / 284.4.
        let right = target_framing(face(700.0, 360.0, 140.0), 1280, 720, 1.8);
        assert!(close(right.pan_x, 60.0 / 284.444), "{right:?}");
        // A face far to the left asks for more than the margin can give: clamped,
        // and the zoom is not raised to chase it.
        let left = target_framing(face(100.0, 360.0, 140.0), 1280, 720, 1.8);
        assert!(close(left.pan_x, -1.0) && close(left.zoom, 1.8));
        let low = target_framing(face(640.0, 700.0, 140.0), 1280, 720, 1.8);
        assert!(close(low.pan_y, 1.0));
    }

    #[test]
    fn a_face_that_can_be_reached_ends_up_at_the_centre_of_the_crop() {
        // A real case: face centre 20% of the half width left of centre.
        let target = target_framing(face(512.0, 305.0, 440.0), 1280, 720, 1.3);
        let crop = 1280.0 / target.zoom;
        let start = (1280.0 - crop) / 2.0 * (1.0 + target.pan_x);
        assert!((start + crop / 2.0 - 512.0).abs() < 1.0, "{target:?}");
    }

    #[test]
    fn there_is_no_framing_until_a_face_has_been_found() {
        let mut reframer = Reframer::new(zoom(1.15), 50);
        assert_eq!(reframer.observe(None, 1280, 720, STEP), None);
        assert_eq!(reframer.observe(None, 1280, 720, STEP), None);
    }

    #[test]
    fn the_first_face_moves_the_pan_partway_and_the_zoom_not_at_all() {
        // Speed 100 is smoothing 0.5: one step covers half the way to the pan.
        let mut reframer = Reframer::new(zoom(1.8), 100);
        let framing = reframer
            .observe(Some(face(700.0, 360.0, 50.0)), 1280, 720, STEP)
            .unwrap();
        assert!(close(framing.zoom, 1.8), "{framing:?}");
        assert!(close(framing.pan_x, 60.0 / 284.444 / 2.0), "{framing:?}");
    }

    #[test]
    fn the_pan_converges_on_the_target_without_overshooting() {
        let mut reframer = Reframer::new(zoom(1.8), 80);
        let mut previous = 0.0;
        for _ in 0..60 {
            let framing = reframer
                .observe(Some(face(700.0, 360.0, 140.0)), 1280, 720, STEP)
                .unwrap();
            assert!(framing.pan_x >= previous && framing.pan_x <= 60.0 / 284.444 + 1e-4);
            assert!(close(framing.zoom, 1.8));
            previous = framing.pan_x;
        }
        assert!(close(previous, 60.0 / 284.444));
    }

    #[test]
    fn a_missed_detection_holds_the_last_framing() {
        let mut reframer = Reframer::new(zoom(1.15), 100);
        let locked = reframer
            .observe(Some(face(640.0, 360.0, 140.0)), 1280, 720, STEP)
            .unwrap();
        assert_eq!(reframer.observe(None, 1280, 720, STEP), Some(locked));
        assert_eq!(reframer.observe(None, 1280, 720, 10 * STEP), Some(locked));
    }

    #[test]
    fn the_smoothing_depends_on_elapsed_time_not_on_how_often_it_is_fed() {
        let target = face(700.0, 360.0, 140.0);
        let mut whole = Reframer::new(zoom(1.8), 100);
        let one = whole.observe(Some(target), 1280, 720, STEP).unwrap();
        let mut halves = Reframer::new(zoom(1.8), 100);
        halves.observe(Some(target), 1280, 720, STEP / 2).unwrap();
        let two = halves.observe(Some(target), 1280, 720, STEP / 2).unwrap();
        assert!(close(one.pan_x, two.pan_x), "{one:?} vs {two:?}");
    }

    #[test]
    fn a_zero_elapsed_time_does_not_break_the_maths() {
        let mut reframer = Reframer::new(zoom(1.8), 100);
        let framing = reframer
            .observe(Some(face(700.0, 360.0, 140.0)), 1280, 720, 0)
            .unwrap();
        assert!(framing.pan_x.is_finite() && framing.pan_x.abs() < 1e-3);
    }

    #[test]
    fn the_zoom_applies_at_once_and_the_speed_can_change_while_running() {
        let mut reframer = Reframer::new(zoom(1.8), 100);
        reframer.set_min_zoom(zoom(1.5));
        reframer.set_speed(0);
        let framing = reframer
            .observe(Some(face(700.0, 360.0, 500.0)), 1280, 720, STEP)
            .unwrap();
        // The zoom is the new one immediately; speed 0 is smoothing 0.95, so
        // the pan covers only 5% of the way.
        assert!(close(framing.zoom, 1.5), "{framing:?}");
        let crop = 1280.0 / 1.5;
        let margin = (1280.0 - crop) / 2.0;
        let wanted = (700.0 - crop / 2.0 - margin) / margin;
        assert!(close(framing.pan_x, wanted * 0.05), "{framing:?}");
    }

    #[test]
    fn the_framing_converts_to_validated_stage_parameters() {
        let framing = AutoFraming {
            zoom: 1.4,
            pan_x: -0.3,
            pan_y: 0.8,
        };
        let (zoom, pan_x, pan_y) = framing.as_params();
        assert_eq!((zoom.get(), pan_x.get(), pan_y.get()), (1.4, -0.3, 0.8));
        // Rounding noise just outside the range is clamped instead of failing.
        let noisy = AutoFraming {
            zoom: 2.0000002,
            pan_x: -1.0000001,
            pan_y: 0.0,
        };
        let (zoom, pan_x, _) = noisy.as_params();
        assert_eq!((zoom.get(), pan_x.get()), (2.0, -1.0));
    }

    #[test]
    fn easing_reaches_the_target_by_elapsed_time_not_by_frame_count() {
        let from = AutoFraming {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        };
        let to = AutoFraming {
            zoom: 2.0,
            pan_x: 1.0,
            pan_y: -1.0,
        };
        let once = from.eased_towards(to, 66_667);
        let twice = from.eased_towards(to, 33_333).eased_towards(to, 33_334);
        assert!(close(once.zoom, twice.zoom) && close(once.pan_x, twice.pan_x));
        assert!(once.zoom > 1.0 && once.zoom < 2.0);
        assert!(from.eased_towards(to, 2_000_000).zoom > 1.99);
        assert_eq!(from.eased_towards(to, 0), from);
    }

    #[test]
    fn a_face_beyond_the_margin_is_reported_as_out_of_reach() {
        // Zoom 1.5 on 1280: margin 213 px each side.
        assert!(within_reach(face(640.0, 360.0, 100.0), 1280, 720, 1.5));
        assert!(within_reach(
            face(640.0 - 200.0, 360.0, 100.0),
            1280,
            720,
            1.5
        ));
        assert!(!within_reach(
            face(640.0 - 260.0, 360.0, 100.0),
            1280,
            720,
            1.5
        ));
        assert!(!within_reach(face(640.0, 20.0, 100.0), 1280, 720, 1.2));
        // No zoom, no margin to speak of: nothing to report.
        assert!(within_reach(face(10.0, 10.0, 50.0), 1280, 720, 1.0));
    }

    #[test]
    fn the_reframer_remembers_whether_the_last_face_was_reachable() {
        let mut reframer = Reframer::new(zoom(1.5), 100);
        assert!(reframer.reachable());
        reframer.observe(Some(face(100.0, 360.0, 100.0)), 1280, 720, STEP);
        assert!(!reframer.reachable());
        // The detections are averaged, so the face has to stay a few frames.
        for _ in 0..12 {
            reframer.observe(Some(face(640.0, 360.0, 100.0)), 1280, 720, STEP);
        }
        assert!(reframer.reachable());
    }

    #[test]
    fn the_zoom_stays_the_chosen_one_even_for_a_face_out_of_reach() {
        let mut reframer = Reframer::new(zoom(1.26), 63);
        let mut framing = None;
        for _ in 0..60 {
            framing = reframer.observe(Some(face(400.0, 305.0, 440.0)), 1280, 720, STEP);
        }
        let framing = framing.unwrap();
        assert!((framing.zoom - 1.26).abs() < 1e-4);
        assert!(!reframer.reachable());
    }

    #[test]
    fn the_detectors_noise_does_not_move_the_framing_but_a_real_move_does() {
        let mut reframer = Reframer::new(zoom(1.3), 63);
        let noise = [0.0, 6.0, -5.0, 4.0, -7.0, 3.0, -4.0, 6.0, -6.0, 2.0];
        let mut pans = Vec::new();
        for step in 0..60 {
            let jitter = noise[step % noise.len()];
            let f = face(700.0 + jitter, 360.0 - jitter, 300.0);
            pans.push(reframer.observe(Some(f), 1280, 720, STEP).unwrap().pan_x);
        }
        let settled = &pans[30..];
        let spread = settled.iter().cloned().fold(f32::MIN, f32::max)
            - settled.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread < 0.02, "the pan trembled by {spread}");
        // Now the person really moves 200 px to the right: the framing follows.
        let before = *pans.last().unwrap();
        let mut after = before;
        for _ in 0..60 {
            after = reframer
                .observe(Some(face(900.0, 360.0, 300.0)), 1280, 720, STEP)
                .unwrap()
                .pan_x;
        }
        assert!(after > before + 0.3, "{before} -> {after}");
    }

    #[test]
    fn the_held_position_is_dragged_by_the_excess_only_so_it_never_jumps() {
        let mut reframer = Reframer::new(zoom(1.3), 100);
        reframer.observe(Some(face(600.0, 360.0, 300.0)), 1280, 720, STEP);
        // The detection is averaged first (600 + 0.4 x 160 = 664), which is 64 px
        // away: 32 px beyond the dead zone, so the aim moves by 32 px, not 160.
        let held = reframer.hold(face(760.0, 360.0, 300.0), 1280, 720);
        assert!((held.center_x - 632.0).abs() < 1e-3, "{held:?}");
    }
}
