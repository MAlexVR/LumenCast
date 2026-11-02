// In-memory stand-in for the Rust backend so the UI runs in a plain browser.
// It mimics the real ordering: negotiated, frames, stats, ended.
import type { Api } from "../api";
import { DEFAULT_AUDIO } from "../logic/audio";
import { DEFAULT_PROCESSING } from "../logic/framing";
import type {
  AppEvent,
  AudioDevice,
  AudioSettings,
  CameraInfo,
  CameraProbe,
  CaptureMode,
  NegotiationReport,
  Preferences,
  ProcessingSettings,
  StatsSnapshot,
  UpdateProgress,
} from "../types";

const mode = (format: string, width: number, height: number, den: number): CaptureMode => ({
  format,
  size: { width, height },
  interval: { numerator: 1, denominator: den },
});

const CAMERAS: CameraInfo[] = [
  { path: "/dev/video0", index: 0, name: "Integrated Camera", usbId: "5986:115f" },
  { path: "/dev/video2", index: 2, name: "SAMSUNG VD USB HDCAM", usbId: "04e8:205d" },
];

const AUDIO_DEVICES: AudioDevice[] = [
  { name: "alsa_input.usb-SAMSUNG_VD_USB_HDCAM.analog-stereo", description: "SAMSUNG VD USB HDCAM Analog Stereo", kind: "source" },
  { name: "alsa_input.pci-0000_06_00.6.analog-stereo", description: "Built-in Audio Analog Stereo", kind: "source" },
  { name: "alsa_output.pci-0000_06_00.6.analog-stereo", description: "Built-in Audio Analog Stereo", kind: "sink" },
];

const PROBE: CameraProbe = {
  capabilities: {
    modes: [
      mode("MJPG", 640, 480, 30),
      mode("MJPG", 1280, 720, 30),
      mode("MJPG", 2560, 1920, 15),
      mode("YUYV", 640, 480, 30),
      mode("YUYV", 1280, 720, 30),
      mode("YUYV", 1920, 1080, 30),
    ],
  },
  skipped: [],
};

function sameMode(a: CaptureMode, b: CaptureMode): boolean {
  return (
    a.format === b.format &&
    a.size.width === b.size.width &&
    a.size.height === b.size.height &&
    a.interval.numerator * b.interval.denominator === b.interval.numerator * a.interval.denominator
  );
}

/** A moving test pattern, so the preview visibly updates. Browser only. */
function patternFrame(tick: number, size: CaptureMode["size"], processing: ProcessingSettings): string {
  const canvas = document.createElement("canvas");
  canvas.width = Math.min(size.width, 640);
  canvas.height = Math.round((canvas.width * size.height) / size.width);
  const ctx = canvas.getContext("2d");
  if (!ctx) return "";
  const style = getComputedStyle(document.documentElement);
  const token = (name: string) => style.getPropertyValue(name).trim();
  ctx.fillStyle = token("--surface-3");
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.fillStyle = token("--moon");
  // The mock only imitates two effects: the preview mirror and a wider bar for zoom.
  const barWidth = 40 * (processing.zoomPercent / 100);
  const travelled = (tick * 7) % canvas.width;
  const x = processing.mirrorPreview ? canvas.width - travelled - barWidth : travelled;
  ctx.fillRect(x, 0, barWidth, canvas.height);
  ctx.strokeStyle = token("--lumen");
  ctx.strokeRect(0.5, 0.5, canvas.width - 1, canvas.height - 1);
  return canvas.toDataURL("image/jpeg", 0.7).split(",")[1] ?? "";
}

export function createMockApi(): Api {
  const listeners = new Set<(event: AppEvent) => void>();
  const updateListeners = new Set<(progress: UpdateProgress) => void>();
  const emit = (event: AppEvent) => listeners.forEach((listener) => listener(event));
  let timer: ReturnType<typeof setInterval> | null = null;
  let finish: (() => void) | null = null;
  let processing: ProcessingSettings = { ...DEFAULT_PROCESSING };
  let autostart = false;
  let audio: AudioSettings = structuredClone(DEFAULT_AUDIO);
  let levelTimer: ReturnType<typeof setInterval> | null = null;
  const audioStatus = (): AppEvent => ({
    kind: "audioStatus",
    microphone: audio.microphone.enabled ? { state: "running" } : { state: "off" },
    speakers: audio.speakers.enabled ? { state: "running" } : { state: "off" },
  });
  /** A moving level on every running path, so the meters visibly work. Browser only. */
  const syncLevels = () => {
    const anyRunning = audio.microphone.enabled || audio.speakers.enabled;
    if (anyRunning && !levelTimer) {
      let tick = 0;
      levelTimer = setInterval(() => {
        tick += 1;
        const wave = (offset: number) => -40 + 25 * Math.abs(Math.sin((tick + offset) / 4));
        if (audio.microphone.enabled) emit({ kind: "audioLevels", path: "microphone", inputDb: wave(0), outputDb: wave(0) - 6 });
        if (audio.speakers.enabled) emit({ kind: "audioLevels", path: "speakers", inputDb: wave(3), outputDb: wave(3) - 6 });
      }, 100);
    } else if (!anyRunning && levelTimer) {
      clearInterval(levelTimer);
      levelTimer = null;
    }
  };
  let language: "es" | "en" | null = null;
  // The mock imitates a first run: the import report is delivered once.
  let importPending = true;

  return {
    listCameras: async () => CAMERAS,
    probeCamera: async (index) => {
      if (!CAMERAS.some((camera) => camera.index === index)) throw { code: "device_not_found", message: "no camera" };
      return PROBE;
    },
    startCapture: async (index, requested) => {
      if (timer) throw { code: "capture_already_running", message: "a capture is already running" };
      if (!CAMERAS.some((camera) => camera.index === index)) throw { code: "device_not_found", message: "no camera" };
      if (!PROBE.capabilities.modes.some((m) => sameMode(m, requested))) {
        setTimeout(() => emit({ kind: "ended", outcome: null, error: { code: "mode_unsupported", message: "unsupported mode" } }), 0);
        return;
      }
      const report: NegotiationReport = { requested, negotiated: requested, differences: [] };
      let captured = 0;
      let previewed = 0;
      const snapshot = (): StatsSnapshot => ({
        framesCaptured: captured,
        framesPreviewed: previewed,
        acquisitionErrors: 0,
        decodeErrors: captured > 0 ? 1 : 0,
        outOfOrderTimestamps: 0,
        measuredFps: captured > 1 ? 30.06 : null,
      });
      setTimeout(() => emit({ kind: "negotiated", report }), 0);
      timer = setInterval(() => {
        captured += 1;
        if (captured % 2 === 1) {
          previewed += 1;
          emit({ kind: "frame", timestampUs: captured * 33_333, jpegBase64: patternFrame(captured, requested.size, processing) });
        }
        if (captured % 15 === 1) emit({ kind: "stats", stats: snapshot() });
        if (captured % 30 === 1 && processing.autoFraming.enabled) {
          // The mock pretends to find a face after the first second.
          emit({
            kind: "autoFraming",
            status:
              captured > 30
                ? { state: "locked", zoomPercent: Math.max(115, processing.autoFraming.minZoomPercent), panX: 0.4, panY: 0.2, reachable: true }
                : { state: "searching" },
          });
        }
      }, 33);
      finish = () => {
        emit({ kind: "ended", outcome: { report, stats: snapshot(), end: { kind: "cancelled" } }, error: null });
      };
    },
    stopCapture: async () => {
      if (!timer) throw { code: "capture_not_running", message: "no capture is running" };
      clearInterval(timer);
      timer = null;
      finish?.();
      finish = null;
    },
    getProcessing: async () => ({ ...processing }),
    setProcessing: async (settings) => {
      const inRange = (value: number, min: number, max: number) => Number.isFinite(value) && value >= min && value <= max;
      if (
        !inRange(settings.zoomPercent, 100, 200) ||
        !inRange(settings.panX, -1, 1) ||
        !inRange(settings.panY, -1, 1) ||
        !inRange(settings.vignette, 0, 1) ||
        !inRange(settings.autoFraming.minZoomPercent, 100, 200) ||
        !inRange(settings.autoFraming.speed, 0, 100)
      ) {
        throw { code: "invalid_setting", message: "setting out of range" };
      }
      processing = { ...settings };
    },
    getAudio: async () => JSON.parse(JSON.stringify(audio)),
    setAudio: async (settings) => {
      const effects = [settings.microphone.noiseRemoval, settings.microphone.roomEchoRemoval, settings.speakers.noiseRemoval, settings.speakers.echoRemoval];
      if (effects.some((effect) => !(effect.strength >= 0 && effect.strength <= 100))) {
        throw { code: "invalid_setting", message: "setting out of range" };
      }
      audio = JSON.parse(JSON.stringify(settings));
      emit(audioStatus());
      syncLevels();
    },
    getAudioStatus: async () => {
      const status = audioStatus();
      return status.kind === "audioStatus" ? { microphone: status.microphone, speakers: status.speakers } : { microphone: { state: "off" }, speakers: { state: "off" } };
    },
    listAudioDevices: async () => AUDIO_DEVICES,
    getPreferences: async (): Promise<Preferences> => {
      const report = importPending
        ? {
            applied: ["language", "mirror_preview", "zoom_factor", "pan_x", "pan_y", "input_device", "resolution", "fps"],
            keptForLater: ["effect_mode", "background_image", "blur_strength", "auto_reframe", "virtual_light"],
            ignored: [{ key: "eye_contact", reason: "Eye Contact was removed from Blucast" }],
          }
        : null;
      importPending = false;
      return {
        settings: {
          version: 1,
          processing: { ...processing },
          language,
          preferredCamera: { usbId: "04e8:205d", name: "SAMSUNG VD USB HDCAM" },
          preferredMode: null,
          modeHint: { width: 1280, height: 720, fps: 30 },
          closeToTray: true,
          checkUpdates: true,
          deferred: {
            effect_mode: "remove",
            background_image: "/home/user/Imágenes/fondo 1.png",
            blur_strength: 50,
            virtual_light: true,
            virtual_light_intensity: 40,
            auto_reframe: true,
            autoreframe_model: "dnn",
            autoreframe_zoom: 135,
            autoreframe_speed: 80,
          },
        },
        importReport: report,
        problem: null,
      };
    },
    setLanguage: async (code) => {
      language = code;
    },
    pickImage: async () => "/home/user/Pictures/fondo.png",
    pickProfileToSave: async (name) => `/home/user/${name}`,
    pickProfileToLoad: async () => "/home/user/lumencast-profile.json",
    saveProfile: async () => {},
    loadProfile: async () => ({ format: "lumencast-profile", version: 1, processing: { ...processing }, audio: JSON.parse(JSON.stringify(audio)) }),
    getAutostart: async () => autostart,
    setAutostart: async (enabled) => {
      autostart = enabled;
    },
    setCloseToTray: async () => {},
    setCheckUpdates: async () => {},
    checkUpdate: async () => ({
      current: "1.0.0",
      install: "rpm",
      available: {
        version: "1.1.0",
        notes: "- Faster background removal\n- Fixes for H264 cameras",
        page: "https://github.com/MAlexVR/LumenCast/releases/tag/v1.1.0",
        published: "2026-11-20T12:00:00Z",
        size: 48_000_000,
        canInstall: true,
      },
    }),
    installUpdate: async () => {
      for (const phase of ["downloading", "verifying", "installing"] as const) {
        updateListeners.forEach((listener) => listener({ phase, done: phase === "downloading" ? 24_000_000 : 0, total: phase === "downloading" ? 48_000_000 : 0 }));
        await new Promise((resolve) => setTimeout(resolve, 300));
      }
    },
    cancelUpdate: async () => {},
    onUpdateProgress: async (handler) => {
      updateListeners.add(handler);
      return () => updateListeners.delete(handler);
    },
    onTrayChanged: async () => () => {},
    onTrayUpdates: async () => () => {},
    onCaptureEvent: async (handler) => {
      listeners.add(handler);
      return () => listeners.delete(handler);
    },
  };
}
