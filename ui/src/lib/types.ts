// Wire types. They mirror the JSON the Rust side serializes (camelCase).

export interface FrameSize {
  width: number;
  height: number;
}

export interface FrameInterval {
  numerator: number;
  denominator: number;
}

/** One capability: format, size and interval always travel together. */
export interface CaptureMode {
  format: string;
  size: FrameSize;
  interval: FrameInterval;
}

export interface CameraInfo {
  path: string;
  index: number;
  name: string | null;
  usbId: string | null;
}

export interface SkippedMode {
  format: string;
  size: [number, number] | null;
  reason: string;
}

export interface CameraProbe {
  capabilities: { modes: CaptureMode[] };
  skipped: SkippedMode[];
}

export type ModeField = "format" | "size" | "interval";

export interface NegotiationReport {
  requested: CaptureMode;
  negotiated: CaptureMode;
  differences: ModeField[];
}

export interface StatsSnapshot {
  framesCaptured: number;
  framesPreviewed: number;
  acquisitionErrors: number;
  decodeErrors: number;
  outOfOrderTimestamps: number;
  /** Null until two ordered capture timestamps were observed. */
  measuredFps: number | null;
}

export interface ErrorPayload {
  code: string;
  message: string;
}

export type EndReason =
  | { kind: "cancelled" }
  | { kind: "frameLimit" }
  | { kind: "failed"; error: ErrorPayload };

export interface SessionOutcome {
  report: NegotiationReport;
  stats: StatsSnapshot;
  end: EndReason;
}

export interface AutoFramingSettings {
  enabled: boolean;
  /** 100 to 200: the framing never zooms out less than this while following. */
  minZoomPercent: number;
  /** 0 (slow and smooth) to 100 (fast). */
  speed: number;
}

export type AutoFramingStatus =
  | { state: "off" }
  | { state: "searching" }
  | { state: "unavailable" }
  | { state: "locked"; zoomPercent: number; panX: number; panY: number; reachable: boolean };

/** Mirrors `ProcessingSettings` in the Rust application layer. */
export type BackgroundMode = "none" | "blur" | "remove" | "replace";

/** `nvidia` is NVIDIA's model on the GPU (in the user's container); `builtin` runs on the CPU. */
export type BackgroundEngine = "nvidia" | "builtin";

export interface BackgroundSettings {
  engine: BackgroundEngine;
  mode: BackgroundMode;
  /** 0 to 100. */
  blurStrength: number;
  /** PNG or JPEG used by `replace`. */
  imagePath: string | null;
}

export type BackgroundProblem = "noPicture" | "pictureUnreadable" | "modelUnavailable";

export type BackgroundStatus =
  | { state: "off" }
  | { state: "working"; engine: BackgroundEngine; fallback: string | null }
  | { state: "unavailable"; problem: BackgroundProblem; reason: string };

/** Cleans low-light noise. */
export interface DenoiseSettings {
  enabled: boolean;
  /** 0 to 100. */
  strength: number;
}

/** Sends the processed video to other programs as a virtual camera. */
export interface VirtualCameraSettings {
  enabled: boolean;
  /** Scale the video that goes out up to this height; 0 leaves it as it is. */
  upscaleHeight: number;
}

export type VirtualCameraStatus =
  | { state: "off" }
  | { state: "active"; device: string; width: number; height: number }
  | { state: "unavailable"; reason: string };

/** A soft light aimed at the face. */
export interface KeyLightSettings {
  enabled: boolean;
  /** 0 to 100. */
  intensity: number;
  /** 0 (cool) to 100 (warm). */
  warmth: number;
  /** A picture of a light (HDR): the light comes from where it is brightest. */
  lightPicture: string | null;
}

export interface ProcessingSettings {
  /** 100 to 200; 100 means no zoom. */
  zoomPercent: number;
  /** -1 to 1. */
  panX: number;
  panY: number;
  /** Flips only the local preview, never the transmitted video. */
  mirrorPreview: boolean;
  /** 0 to 1; 0 means off. */
  vignette: number;
  /** Centre the vignette on the face instead of the middle of the picture. */
  vignetteFollowsFace: boolean;
  autoFraming: AutoFramingSettings;
  keyLight: KeyLightSettings;
  background: BackgroundSettings;
  denoise: DenoiseSettings;
  virtualCamera: VirtualCameraSettings;
}

/** One audio effect: on or off, and how hard it works (0 to 100). */
export interface AudioEffect {
  enabled: boolean;
  strength: number;
}

export type MicProfile = "headset" | "laptop" | "desk";

export interface MicrophoneSettings {
  /** Publishes the virtual microphone. */
  enabled: boolean;
  /** The real microphone; `null` is the system default. */
  source: string | null;
  noiseRemoval: AudioEffect;
  roomEchoRemoval: AudioEffect;
  studioVoice: boolean;
  profile: MicProfile;
}

export interface SpeakerSettings {
  /** Publishes the virtual output. */
  enabled: boolean;
  /** The real output; `null` is the system default. */
  sink: string | null;
  noiseRemoval: AudioEffect;
  echoRemoval: AudioEffect;
}

export interface AudioSettings {
  microphone: MicrophoneSettings;
  speakers: SpeakerSettings;
}

export interface AudioDevice {
  name: string;
  description: string;
  kind: "source" | "sink";
}

export type PathStatus = { state: "off" } | { state: "running" } | { state: "unavailable"; reason: string };

export type AudioPath = "microphone" | "speakers";

export interface AudioPaths {
  microphone: PathStatus;
  speakers: PathStatus;
}

export interface PreferredCamera {
  usbId: string | null;
  name: string | null;
}

/** A size and frame rate remembered from the legacy settings; only a hint. */
export interface ModeHint {
  width: number;
  height: number;
  fps: number;
}

export interface PersistedSettings {
  version: number;
  processing: ProcessingSettings;
  language: "es" | "en" | null;
  preferredCamera: PreferredCamera | null;
  preferredMode: CaptureMode | null;
  modeHint: ModeHint | null;
  /** Legacy effect settings kept for later; opaque to the UI. */
  deferred: Record<string, unknown>;
  /** Closing the window keeps the program running in the system tray. */
  closeToTray: boolean;
  /** Look for a new version when the program starts. */
  checkUpdates: boolean;
}

export type InstallKind = "rpm" | "deb" | "appImage" | "unmanaged";

export interface AvailableUpdate {
  version: string;
  notes: string;
  page: string;
  published: string;
  /** Bytes of the installer; 0 when there is none for this system. */
  size: number;
  /** Whether the program can install it by itself. */
  canInstall: boolean;
}

export interface UpdateStatus {
  current: string;
  install: InstallKind;
  available: AvailableUpdate | null;
}

export type UpdatePhase = "downloading" | "verifying" | "installing";

export interface UpdateProgress {
  phase: UpdatePhase;
  done: number;
  total: number;
}

export interface IgnoredSetting {
  key: string;
  reason: string;
}

export interface ImportReport {
  applied: string[];
  keptForLater: string[];
  ignored: IgnoredSetting[];
}

export interface Preferences {
  settings: PersistedSettings;
  /** Present only once, right after the settings were imported. */
  importReport: ImportReport | null;
  /** Set when the saved settings could not be read; defaults are in use. */
  problem: string | null;
}

export type AppEvent =
  | { kind: "negotiated"; report: NegotiationReport }
  | { kind: "frame"; timestampUs: number; jpegBase64: string }
  | { kind: "stats"; stats: StatsSnapshot }
  | { kind: "previewDropped"; total: number }
  | { kind: "autoFraming"; status: AutoFramingStatus }
  | { kind: "background"; status: BackgroundStatus }
  | { kind: "virtualCamera"; status: VirtualCameraStatus }
  | { kind: "audioStatus"; microphone: PathStatus; speakers: PathStatus }
  | { kind: "audioLevels"; path: AudioPath; inputDb: number; outputDb: number }
  | { kind: "ended"; outcome: SessionOutcome | null; error: ErrorPayload | null };
