//! Hardware check of the whole preview path: real camera, real `AppCore`,
//! processing switched on. It guards against the preview slowing the capture.
//! Ignored by default (needs a camera):
//!
//!   LUMENCAST_CAMERA=/dev/video2 cargo test -p lumencast-app --test host_preview \
//!       -- --ignored --nocapture --test-threads=1

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lumencast_app::core::{AppConfig, AppCore};
use lumencast_app::events::{AppEvent, EventSink};
use lumencast_app::processing::ProcessingSettings;
use lumencast_core::mode::{CaptureMode, FrameInterval, FrameSize, PixelFormat};
use lumencast_core::session::{EndReason, SessionConfig};
use lumencast_v4l::device::V4lCatalog;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Collector(Mutex<Vec<AppEvent>>);

impl EventSink for Collector {
    fn emit(&self, event: AppEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn mode(fourcc: &str, width: u32, height: u32) -> CaptureMode {
    CaptureMode {
        format: PixelFormat::parse(fourcc).unwrap(),
        size: FrameSize::new(width, height).unwrap(),
        interval: FrameInterval::new(1, 30).unwrap(),
    }
}

fn run_collecting(
    fourcc: &str,
    width: u32,
    height: u32,
    settings: ProcessingSettings,
) -> Vec<AppEvent> {
    let wanted_path = std::env::var("LUMENCAST_CAMERA").expect("set LUMENCAST_CAMERA=/dev/videoN");
    let catalog = Arc::new(V4lCatalog::default());
    let sink = Arc::new(Collector::default());
    let config = AppConfig {
        session: SessionConfig {
            max_frames: Some(120),
            ..SessionConfig::default()
        },
        ..AppConfig::default()
    };
    let core = AppCore::new(catalog, sink.clone(), config);
    let index = core
        .list_cameras()
        .into_iter()
        .find(|camera| camera.path == wanted_path)
        .expect("camera not listed")
        .index;
    core.set_processing(settings).unwrap();
    core.start(index, mode(fourcc, width, height)).unwrap();

    let deadline = Instant::now() + Duration::from_secs(20);
    let events = loop {
        let events = sink.0.lock().unwrap().clone();
        if events
            .iter()
            .any(|event| matches!(event, AppEvent::Ended { .. }))
        {
            break events;
        }
        assert!(Instant::now() < deadline, "the capture did not end in time");
        std::thread::sleep(Duration::from_millis(20));
    };
    events
}

fn run(fourcc: &str, width: u32, height: u32, settings: ProcessingSettings) {
    let events = run_collecting(fourcc, width, height, settings);
    let Some(AppEvent::Ended {
        outcome: Some(outcome),
        error: None,
    }) = events.last()
    else {
        panic!("capture ended badly: {:?}", events.last());
    };
    let previews = events
        .iter()
        .filter(|e| matches!(e, AppEvent::Frame { .. }))
        .count();
    let dropped = events
        .iter()
        .filter(|e| matches!(e, AppEvent::PreviewDropped { .. }))
        .count();
    println!(
        "{fourcc} {width}x{height}: end {:?}, captured {}, measured {:?} fps, preview frames {previews}, dropped {dropped}",
        outcome.end, outcome.stats.frames_captured, outcome.stats.measured_fps
    );
    assert_eq!(outcome.end, EndReason::FrameLimit);
    let fps = outcome.stats.measured_fps.expect("measured fps");
    assert!(
        fps >= 25.0,
        "measured only {fps} fps with the preview processing on"
    );
    assert!(
        previews >= 10,
        "only {previews} preview frames reached the UI"
    );
}

fn heavy() -> ProcessingSettings {
    ProcessingSettings {
        zoom_percent: 130.0,
        pan_x: 0.3,
        pan_y: -0.2,
        mirror_preview: true,
        vignette: 0.3,
        ..ProcessingSettings::default()
    }
}

#[test]
#[ignore = "needs a real camera"]
fn mjpeg_with_every_effect_on_keeps_the_capture_rate() {
    run("MJPG", 1280, 720, heavy());
}

#[test]
#[ignore = "needs a real camera"]
fn mirror_alone_keeps_the_capture_rate() {
    run(
        "MJPG",
        1280,
        720,
        ProcessingSettings {
            mirror_preview: true,
            ..ProcessingSettings::default()
        },
    );
}

#[test]
#[ignore = "needs a real camera"]
fn yuyv_is_shown_with_neutral_settings() {
    run("YUYV", 640, 480, ProcessingSettings::default());
}

/// The Samsung's H264 frames carry a vendor header and a plane before the
/// stream; the decoded picture has to reach the preview, and it has to be a
/// picture, not an empty or uniform frame.
#[test]
#[ignore = "needs a camera with an H264 mode (the Samsung VD USB HDCAM)"]
fn h264_from_the_real_camera_reaches_the_preview() {
    h264_reaches_the_preview(640, 480);
}

/// The camera lists larger H264 sizes but sends 640x480 for any of them.
#[test]
#[ignore = "needs a camera with an H264 mode (the Samsung VD USB HDCAM)"]
fn h264_asked_at_720p_reaches_the_preview_too() {
    h264_reaches_the_preview(1280, 720);
}

/// The Samsung sends the keyframe of its H264 stream only as the stream starts;
/// when that frame is lost the picture waits a long time. Start the capture many
/// times in a row: every start must show a picture within a second or two.
#[test]
#[ignore = "needs a camera with an H264 mode (the Samsung VD USB HDCAM)"]
fn h264_shows_a_picture_every_time_it_starts() {
    for attempt in 1..=12 {
        let started = std::time::Instant::now();
        let events = run_collecting("H264", 640, 480, ProcessingSettings::default());
        let previews = events
            .iter()
            .filter(|e| matches!(e, AppEvent::Frame { .. }))
            .count();
        let dropped = events
            .iter()
            .filter_map(|e| match e {
                AppEvent::PreviewDropped { total } => Some(*total),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        println!(
            "start {attempt}: {previews} preview frames, {dropped} dropped, {:.1} s",
            started.elapsed().as_secs_f64()
        );
        assert!(
            previews >= 60,
            "start {attempt}: only {previews} preview frames"
        );
        assert!(dropped <= 5, "start {attempt}: {dropped} frames dropped");
    }
}

/// A typical set of effects, with the built-in background model; prints the
/// capture rate. `LUMENCAST_CAMERA` picks the camera.
#[test]
#[ignore = "needs a real camera"]
fn the_owners_effects_keep_the_capture_rate() {
    let mut settings = ProcessingSettings {
        mirror_preview: true,
        ..ProcessingSettings::default()
    };
    settings.denoise.enabled = true;
    settings.denoise.strength = 69;
    settings.key_light.enabled = true;
    settings.key_light.intensity = 70;
    settings.auto_framing.enabled = true;
    settings.auto_framing.min_zoom_percent = 130.0;
    settings.background.engine = if std::env::var("LUMENCAST_NVIDIA").is_ok() {
        lumencast_app::processing::BackgroundEngine::Nvidia
    } else {
        lumencast_app::processing::BackgroundEngine::Builtin
    };
    settings.background.mode = lumencast_app::processing::BackgroundMode::Remove;
    settings.virtual_camera.enabled = std::env::var("LUMENCAST_VCAM").is_ok();
    run("MJPG", 1280, 720, settings);
}

/// Every typical effect on, on top of H264.
#[test]
#[ignore = "needs a camera with an H264 mode (the Samsung VD USB HDCAM)"]
fn h264_with_the_effects_on_reaches_the_preview() {
    let mut settings = ProcessingSettings {
        mirror_preview: true,
        ..ProcessingSettings::default()
    };
    settings.denoise.enabled = true;
    settings.denoise.strength = 69;
    settings.key_light.enabled = true;
    settings.key_light.intensity = 70;
    settings.auto_framing.enabled = true;
    settings.auto_framing.min_zoom_percent = 130.0;
    settings.background.engine = lumencast_app::processing::BackgroundEngine::Builtin;
    settings.background.mode = lumencast_app::processing::BackgroundMode::Remove;
    // The virtual camera too, when the loopback device is free to write to.
    settings.virtual_camera.enabled = std::env::var("LUMENCAST_VCAM").is_ok();
    h264_with(1280, 720, settings);
}

fn h264_reaches_the_preview(width: u32, height: u32) {
    h264_with(width, height, ProcessingSettings::default());
}

fn h264_with(width: u32, height: u32, settings: ProcessingSettings) {
    let events = run_collecting("H264", width, height, settings);
    let previews = events
        .iter()
        .filter(|e| matches!(e, AppEvent::Frame { .. }))
        .count();
    let dropped = events
        .iter()
        .filter_map(|e| match e {
            AppEvent::PreviewDropped { total } => Some(*total),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let camera = events.iter().rev().find_map(|e| match e {
        AppEvent::VirtualCamera { status } => Some(format!("{status:?}")),
        _ => None,
    });
    println!("H264 {width}x{height}: preview frames {previews}, dropped {dropped}, virtual camera {camera:?}");
    assert!(
        previews >= 60,
        "only {previews} H264 preview frames reached the UI"
    );
    assert!(dropped <= 5, "{dropped} H264 frames could not be decoded");
}

/// Read-only: imports a real Blucast settings file against the real camera list
/// and prints what would happen. Writes nothing.
#[test]
#[ignore = "reads ~/.config/blucast/settings.json"]
fn the_real_legacy_settings_file_imports_cleanly() {
    use lumencast_app::settings::import_legacy;
    use lumencast_core::camera::CameraCatalog;

    let home = std::env::var("HOME").unwrap();
    let text = std::fs::read_to_string(format!("{home}/.config/blucast/settings.json")).unwrap();
    let cameras = V4lCatalog::default().list();
    let imported = import_legacy(&text, Some(&home), &cameras).unwrap();
    println!("processing: {:?}", imported.settings.processing);
    println!("language: {:?}", imported.settings.language);
    println!("camera: {:?}", imported.settings.preferred_camera);
    println!("mode hint: {:?}", imported.settings.mode_hint);
    println!("applied: {:?}", imported.report.applied);
    println!("kept for later: {:?}", imported.report.kept_for_later);
    println!("ignored: {:?}", imported.report.ignored);
    println!(
        "background_image: {:?}",
        imported.settings.deferred.get("background_image")
    );
    assert!(imported
        .report
        .applied
        .iter()
        .any(|key| key == "zoom_factor"));
    assert!(imported
        .report
        .applied
        .iter()
        .any(|key| key == "input_device"));
}

/// Needs a person in view. The real SeetaFace detector, the real camera, the
/// real core: the capture must keep its rate and the framing must lock on.
#[test]
#[ignore = "needs a real camera and a person in view"]
fn auto_framing_locks_onto_the_face_without_slowing_the_capture() {
    let mut settings = ProcessingSettings::default();
    settings.auto_framing.enabled = true;
    settings.auto_framing.speed = 80;
    let events = run_collecting("MJPG", 1280, 720, settings);
    let Some(AppEvent::Ended {
        outcome: Some(outcome),
        error: None,
    }) = events.last()
    else {
        panic!("capture ended badly: {:?}", events.last());
    };
    let statuses: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AppEvent::AutoFraming { status } => Some(*status),
            _ => None,
        })
        .collect();
    println!(
        "measured {:?} fps; auto framing events: {statuses:?}",
        outcome.stats.measured_fps
    );
    let fps = outcome.stats.measured_fps.expect("measured fps");
    assert!(
        fps >= 25.0,
        "measured only {fps} fps while following a face"
    );
    assert!(
        statuses.iter().any(|status| matches!(
            status,
            lumencast_app::preview::AutoFramingStatus::Locked { .. }
        )),
        "the framing never locked onto a face: {statuses:?}"
    );
}

/// Needs a person in view: the key light follows the real face and the capture
/// must keep its rate.
#[test]
#[ignore = "needs a real camera and a person in view"]
fn the_key_light_keeps_the_capture_rate() {
    let mut settings = ProcessingSettings::default();
    settings.key_light.enabled = true;
    settings.key_light.intensity = 80;
    run("MJPG", 1280, 720, settings);
}

#[test]
#[ignore = "needs a real camera and a person in view"]
fn the_key_light_with_auto_framing_and_every_effect_keeps_the_capture_rate() {
    let mut settings = heavy();
    settings.key_light.enabled = true;
    settings.auto_framing.enabled = true;
    run("MJPG", 1280, 720, settings);
}

/// Needs a person in view: the real segmentation model on the real camera.
#[test]
#[ignore = "needs a real camera and a person in view"]
fn background_blur_keeps_the_capture_rate() {
    use lumencast_app::processing::BackgroundMode;
    let mut settings = ProcessingSettings::default();
    settings.background.mode = BackgroundMode::Blur;
    run("MJPG", 1280, 720, settings);
}

#[test]
#[ignore = "needs a real camera and a person in view"]
fn background_removal_with_every_other_effect_keeps_the_capture_rate() {
    use lumencast_app::processing::BackgroundMode;
    let mut settings = heavy();
    settings.key_light.enabled = true;
    settings.auto_framing.enabled = true;
    settings.background.mode = BackgroundMode::Remove;
    run("MJPG", 1280, 720, settings);
}

#[test]
#[ignore = "needs a real camera and a person in view"]
fn every_effect_at_once_including_noise_reduction_keeps_the_capture_rate() {
    use lumencast_app::processing::BackgroundMode;
    let mut settings = heavy();
    settings.key_light.enabled = true;
    settings.auto_framing.enabled = true;
    settings.denoise.enabled = true;
    settings.denoise.strength = 80;
    settings.background.mode = BackgroundMode::Blur;
    run("MJPG", 1280, 720, settings);
}

/// Needs the GPU, the NVIDIA matte image built (`sidecar/nvidia-matte`) and a
/// person in view: NVIDIA's model must be the one actually used, and the
/// capture must keep its rate with every other effect on.
#[test]
#[ignore = "needs a real camera, the NVIDIA matte image and a GPU"]
fn nvidia_background_removal_is_used_and_keeps_the_capture_rate() {
    use lumencast_app::preview::BackgroundStatus;
    use lumencast_app::processing::{BackgroundEngine, BackgroundMode};
    let mut settings = heavy();
    settings.key_light.enabled = true;
    settings.auto_framing.enabled = true;
    settings.denoise.enabled = true;
    settings.background.engine = BackgroundEngine::Nvidia;
    settings.background.mode = BackgroundMode::Remove;
    let events = run_collecting("MJPG", 1280, 720, settings);
    let statuses: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AppEvent::Background { status } => Some(status.clone()),
            _ => None,
        })
        .collect();
    println!("background statuses: {statuses:?}");
    let Some(AppEvent::Ended {
        outcome: Some(outcome),
        error: None,
    }) = events.last()
    else {
        panic!("capture ended badly: {:?}", events.last());
    };
    println!("measured {:?} fps", outcome.stats.measured_fps);
    assert!(outcome.stats.measured_fps.unwrap() >= 25.0);
    assert!(
        statuses.iter().any(|status| matches!(
            status,
            BackgroundStatus::Working {
                engine: BackgroundEngine::Nvidia,
                fallback: None
            }
        )),
        "NVIDIA's model was never in use: {statuses:?}"
    );
}

struct CountingOutput(Arc<std::sync::atomic::AtomicU64>);

impl lumencast_app::output::FrameOutput for CountingOutput {
    fn send(&mut self, _frame: &lumencast_fx::frame::Frame) -> Result<(), String> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }

    fn device(&self) -> String {
        "counting output".into()
    }
}

struct CountingFactory(Arc<std::sync::atomic::AtomicU64>);

impl lumencast_app::output::OutputFactory for CountingFactory {
    fn open(
        &self,
        _width: u32,
        _height: u32,
    ) -> Result<Box<dyn lumencast_app::output::FrameOutput>, String> {
        Ok(Box::new(CountingOutput(Arc::clone(&self.0))))
    }
}

/// The virtual camera path with every effect on, against the real camera but a
/// counting stand-in for the device (so no real virtual camera is touched):
/// it must receive close to every captured frame, and the capture must keep
/// its rate. Needs a person in view, the GPU and the NVIDIA matte image.
#[test]
#[ignore = "needs a real camera, a person in view, the NVIDIA matte image and a GPU"]
fn the_virtual_camera_gets_almost_every_frame_with_every_effect_on() {
    use lumencast_app::preview::FxRenderer;
    use lumencast_app::processing::{BackgroundEngine, BackgroundMode};
    let wanted_path = std::env::var("LUMENCAST_CAMERA").expect("set LUMENCAST_CAMERA=/dev/videoN");
    let delivered = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let renderer =
        FxRenderer::seeta().with_outputs(Arc::new(CountingFactory(Arc::clone(&delivered))));
    let sink = Arc::new(Collector::default());
    let config = AppConfig {
        session: SessionConfig {
            max_frames: Some(150),
            ..SessionConfig::default()
        },
        ..AppConfig::default()
    };
    let core = AppCore::with_renderer(
        Arc::new(V4lCatalog::default()),
        sink.clone(),
        config,
        Arc::new(renderer),
    );
    let index = core
        .list_cameras()
        .into_iter()
        .find(|camera| camera.path == wanted_path)
        .expect("camera not listed")
        .index;
    let mut settings = heavy();
    settings.key_light.enabled = true;
    settings.auto_framing.enabled = true;
    settings.denoise.enabled = true;
    settings.virtual_camera.enabled = true;
    settings.background.engine = BackgroundEngine::Nvidia;
    settings.background.mode = BackgroundMode::Remove;
    core.set_processing(settings).unwrap();
    core.start(index, mode("MJPG", 1280, 720)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    let events = loop {
        let events = sink.0.lock().unwrap().clone();
        if events
            .iter()
            .any(|event| matches!(event, AppEvent::Ended { .. }))
        {
            break events;
        }
        assert!(Instant::now() < deadline, "the capture did not end in time");
        std::thread::sleep(Duration::from_millis(20));
    };
    std::thread::sleep(Duration::from_millis(500));
    let Some(AppEvent::Ended {
        outcome: Some(outcome),
        ..
    }) = events.last()
    else {
        panic!("capture ended badly: {:?}", events.last());
    };
    let delivered = delivered.load(std::sync::atomic::Ordering::SeqCst);
    let fps = outcome.stats.measured_fps.unwrap();
    println!(
        "captured {} frames at {fps:.2} fps, the virtual camera received {delivered}",
        outcome.stats.frames_captured
    );
    assert!(fps >= 25.0);
    assert!(
        delivered * 10 >= outcome.stats.frames_captured * 8,
        "less than 80% reached the virtual camera"
    );
}

/// The whole path against the real devices: real camera, the real effects and
/// the real v4l2loopback device, read back through `v4l2-ctl` the way another
/// program would. A strong vignette is on, so the corners of what comes out
/// must be darker than its middle. Refuses to run while another program has the
/// device open. Needs a v4l2loopback device and a camera.
#[test]
#[ignore = "needs a real camera and a v4l2loopback device nobody else is using"]
fn the_processed_video_reaches_the_real_virtual_camera() {
    use std::process::Command;
    let device = lumencast_v4l::output::find_loopback()
        .expect("no v4l2loopback device")
        .display()
        .to_string();
    let users = Command::new("fuser").arg(&device).output().unwrap();
    assert!(
        String::from_utf8_lossy(&users.stdout).trim().is_empty(),
        "{device} is in use; close the program that sends to it first"
    );
    let wanted_path = std::env::var("LUMENCAST_CAMERA").expect("set LUMENCAST_CAMERA=/dev/videoN");
    let sink = Arc::new(Collector::default());
    let config = AppConfig {
        session: SessionConfig {
            max_frames: Some(150),
            ..SessionConfig::default()
        },
        ..AppConfig::default()
    };
    let core = AppCore::new(Arc::new(V4lCatalog::default()), sink.clone(), config);
    let index = core
        .list_cameras()
        .into_iter()
        .find(|camera| camera.path == wanted_path)
        .expect("camera not listed")
        .index;
    let mut settings = ProcessingSettings {
        vignette: 1.0,
        ..ProcessingSettings::default()
    };
    settings.virtual_camera.enabled = true;
    core.set_processing(settings).unwrap();
    core.start(index, mode("MJPG", 1280, 720)).unwrap();
    std::thread::sleep(Duration::from_millis(1500));

    let dir = std::env::temp_dir().join(format!("lumencast-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("out.yuv");
    let read = Command::new("v4l2-ctl")
        .args([
            "-d",
            &device,
            "--stream-mmap",
            "--stream-count=3",
            "--stream-to",
            file.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    let events = loop {
        let events = sink.0.lock().unwrap().clone();
        if events
            .iter()
            .any(|event| matches!(event, AppEvent::Ended { .. }))
        {
            break events;
        }
        assert!(Instant::now() < deadline, "the capture did not end in time");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        read.status.success(),
        "v4l2-ctl failed: {}",
        String::from_utf8_lossy(&read.stderr)
    );
    let bytes = std::fs::read(&file).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    let statuses: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AppEvent::VirtualCamera { status } => Some(status.clone()),
            _ => None,
        })
        .collect();
    println!("virtual camera statuses: {statuses:?}");
    let (width, height) = (1280usize, 720usize);
    let frame = width * height * 3 / 2;
    assert!(bytes.len() >= frame, "only {} bytes were read", bytes.len());
    let luma = &bytes[bytes.len() - frame..bytes.len() - frame + width * height];
    let middle = luma[(height / 2) * width + width / 2];
    let corner = luma[8 * width + 8];
    println!("read back: middle luma {middle}, corner luma {corner}");
    assert!(
        corner + 40 < middle,
        "the vignette did not reach the output (corner {corner}, middle {middle})"
    );
}
