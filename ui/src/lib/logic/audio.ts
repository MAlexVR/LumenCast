import type { AppEvent, AudioEffect, AudioPath, AudioPaths, AudioSettings, MicrophoneSettings, SpeakerSettings } from "../types";

/** Mirrors `AudioSettings::default()` on the Rust side. */
export const DEFAULT_AUDIO: AudioSettings = {
  microphone: {
    enabled: false,
    source: null,
    noiseRemoval: { enabled: true, strength: 80 },
    roomEchoRemoval: { enabled: false, strength: 50 },
    studioVoice: false,
    profile: "headset",
  },
  speakers: {
    enabled: false,
    sink: null,
    noiseRemoval: { enabled: true, strength: 70 },
    echoRemoval: { enabled: false, strength: 50 },
  },
};

export function withMicrophone(settings: AudioSettings, change: Partial<MicrophoneSettings>): AudioSettings {
  return { ...settings, microphone: { ...settings.microphone, ...change } };
}

export function withSpeakers(settings: AudioSettings, change: Partial<SpeakerSettings>): AudioSettings {
  return { ...settings, speakers: { ...settings.speakers, ...change } };
}

export function withEffect(effect: AudioEffect, change: Partial<AudioEffect>): AudioEffect {
  return { ...effect, ...change };
}

/** The lowest level the meter shows; anything quieter reads as empty. */
export const METER_FLOOR_DB = -60;

/** 0 to 100 for a meter bar: the floor is empty and 0 dB is full. */
export function meterPercent(db: number): number {
  if (!Number.isFinite(db)) return 0;
  const clamped = Math.min(0, Math.max(METER_FLOOR_DB, db));
  return Math.round(((clamped - METER_FLOOR_DB) / -METER_FLOOR_DB) * 100);
}

export interface PathLevels {
  inputDb: number;
  outputDb: number;
}

export interface AudioState extends AudioPaths {
  levels: Record<AudioPath, PathLevels>;
}

const SILENT: PathLevels = { inputDb: -100, outputDb: -100 };

export const initialAudioState: AudioState = {
  microphone: { state: "off" },
  speakers: { state: "off" },
  levels: { microphone: SILENT, speakers: SILENT },
};

/** A path that is not running reports no level, so the meters fall back to empty. */
export function reduceAudio(state: AudioState, event: AppEvent): AudioState {
  switch (event.kind) {
    case "audioStatus":
      return {
        microphone: event.microphone,
        speakers: event.speakers,
        levels: {
          microphone: event.microphone.state === "running" ? state.levels.microphone : SILENT,
          speakers: event.speakers.state === "running" ? state.levels.speakers : SILENT,
        },
      };
    case "audioLevels":
      return {
        ...state,
        levels: { ...state.levels, [event.path]: { inputDb: event.inputDb, outputDb: event.outputDb } },
      };
    default:
      return state;
  }
}

/** Whether the backend can apply the settings as they are: strengths in range. */
export function audioInRange(settings: AudioSettings): boolean {
  const effects = [
    settings.microphone.noiseRemoval,
    settings.microphone.roomEchoRemoval,
    settings.speakers.noiseRemoval,
    settings.speakers.echoRemoval,
  ];
  return effects.every((effect) => Number.isInteger(effect.strength) && effect.strength >= 0 && effect.strength <= 100);
}
