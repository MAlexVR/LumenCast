import type { MessageKey } from "../i18n";
import type { AutoFramingSettings, BackgroundSettings, DenoiseSettings, KeyLightSettings, AutoFramingStatus, ProcessingSettings } from "../types";

export const DEFAULT_PROCESSING: ProcessingSettings = {
  zoomPercent: 100,
  panX: 0,
  panY: 0,
  mirrorPreview: false,
  vignette: 0,
  vignetteFollowsFace: false,
  autoFraming: { enabled: false, minZoomPercent: 130, speed: 35 },
  keyLight: { enabled: false, intensity: 40, warmth: 60, lightPicture: null },
  background: { engine: "nvidia", mode: "none", blurStrength: 50, imagePath: null },
  denoise: { enabled: false, strength: 60 },
  virtualCamera: { enabled: false, upscaleHeight: 0 },
};

/** Same step as the legacy D-pad. */
export const PAN_STEP = 0.1;

export type PanDirection = "up" | "down" | "left" | "right";

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));
/** Two decimals: ten steps of 0.1 land exactly on 1 instead of drifting. */
const tidy = (value: number) => Math.round(value * 100) / 100;

export function nudgePan(settings: ProcessingSettings, direction: PanDirection): ProcessingSettings {
  const step = { up: [0, -PAN_STEP], down: [0, PAN_STEP], left: [-PAN_STEP, 0], right: [PAN_STEP, 0] }[direction];
  return {
    ...settings,
    panX: clamp(tidy(settings.panX + step[0]), -1, 1),
    panY: clamp(tidy(settings.panY + step[1]), -1, 1),
  };
}

export function centerPan(settings: ProcessingSettings): ProcessingSettings {
  return { ...settings, panX: 0, panY: 0 };
}

export function withZoom(settings: ProcessingSettings, percent: number): ProcessingSettings {
  const whole = Number.isFinite(percent) ? Math.round(percent) : 100;
  return { ...settings, zoomPercent: clamp(whole, 100, 200) };
}

/** Panning needs the margin that zooming leaves; at 100% there is none. */
export function canPan(settings: ProcessingSettings): boolean {
  return settings.zoomPercent > 100;
}

export function isNeutral(settings: ProcessingSettings): boolean {
  return settings.zoomPercent <= 100 && !settings.mirrorPreview && settings.vignette === 0 && !settings.autoFraming.enabled && !settings.keyLight.enabled && settings.background.mode === "none" && !settings.denoise.enabled && !settings.virtualCamera.enabled;
}

/** The zoom as the sliders show it: 0 is no zoom and 100 is the most (2x). */
export function zoomLevel(zoomPercent: number): number {
  return clamp(Math.round(zoomPercent) - 100, 0, 100);
}

/** The zoom the backend takes (100 to 200) for a slider level from 0 to 100. */
export function zoomFromLevel(level: number): number {
  return 100 + clamp(Number.isFinite(level) ? Math.round(level) : 0, 0, 100);
}

export function withAutoFraming(settings: ProcessingSettings, patch: Partial<AutoFramingSettings>): ProcessingSettings {
  const current = settings.autoFraming;
  const minZoom = patch.minZoomPercent;
  const speed = patch.speed;
  return {
    ...settings,
    autoFraming: {
      enabled: patch.enabled ?? current.enabled,
      minZoomPercent: minZoom === undefined ? current.minZoomPercent : clamp(Number.isFinite(minZoom) ? Math.round(minZoom) : current.minZoomPercent, 100, 200),
      speed: speed === undefined ? current.speed : clamp(Number.isFinite(speed) ? Math.round(speed) : current.speed, 0, 100),
    },
  };
}

/** The text key for the auto framing state, or null when there is nothing to say. */
export function autoStatusKey(status: AutoFramingStatus | null): MessageKey | null {
  switch (status?.state) {
    case "searching":
      return "auto_searching";
    case "unavailable":
      return "auto_unavailable";
    case "locked":
      return "auto_locked";
    default:
      return null;
  }
}

export function withKeyLight(settings: ProcessingSettings, patch: Partial<KeyLightSettings>): ProcessingSettings {
  const current = settings.keyLight;
  const level = (value: number | undefined, fallback: number) =>
    value === undefined ? fallback : clamp(Number.isFinite(value) ? Math.round(value) : fallback, 0, 100);
  return {
    ...settings,
    keyLight: {
      enabled: patch.enabled ?? current.enabled,
      intensity: level(patch.intensity, current.intensity),
      warmth: level(patch.warmth, current.warmth),
      lightPicture: patch.lightPicture === undefined ? current.lightPicture : patch.lightPicture,
    },
  };
}

export function withBackground(settings: ProcessingSettings, patch: Partial<BackgroundSettings>): ProcessingSettings {
  const current = settings.background;
  const blur = patch.blurStrength;
  return {
    ...settings,
    background: {
      engine: patch.engine ?? current.engine,
      mode: patch.mode ?? current.mode,
      blurStrength: blur === undefined ? current.blurStrength : clamp(Number.isFinite(blur) ? Math.round(blur) : current.blurStrength, 0, 100),
      imagePath: patch.imagePath === undefined ? current.imagePath : patch.imagePath,
    },
  };
}

export function withDenoise(settings: ProcessingSettings, patch: Partial<DenoiseSettings>): ProcessingSettings {
  const current = settings.denoise;
  const strength = patch.strength;
  return {
    ...settings,
    denoise: {
      enabled: patch.enabled ?? current.enabled,
      strength: strength === undefined ? current.strength : clamp(Number.isFinite(strength) ? Math.round(strength) : current.strength, 0, 100),
    },
  };
}
