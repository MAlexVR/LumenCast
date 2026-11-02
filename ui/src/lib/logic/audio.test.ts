import { describe, expect, it } from "vitest";
import { audioInRange, DEFAULT_AUDIO, initialAudioState, meterPercent, reduceAudio, withEffect, withMicrophone, withSpeakers } from "./audio";

describe("audio settings helpers", () => {
  it("change one field and keep the rest", () => {
    const next = withMicrophone(DEFAULT_AUDIO, { studioVoice: true });
    expect(next.microphone.studioVoice).toBe(true);
    expect(next.microphone.noiseRemoval).toEqual(DEFAULT_AUDIO.microphone.noiseRemoval);
    expect(next.speakers).toBe(DEFAULT_AUDIO.speakers);
    expect(withSpeakers(DEFAULT_AUDIO, { sink: "out" }).microphone).toBe(DEFAULT_AUDIO.microphone);
    expect(withEffect({ enabled: true, strength: 10 }, { strength: 20 })).toEqual({ enabled: true, strength: 20 });
  });

  it("start with nothing published and noise removal as the first effect on", () => {
    expect(DEFAULT_AUDIO.microphone.enabled || DEFAULT_AUDIO.speakers.enabled).toBe(false);
    expect(DEFAULT_AUDIO.microphone.noiseRemoval).toEqual({ enabled: true, strength: 80 });
  });

  it("flag a strength outside 0 to 100", () => {
    expect(audioInRange(DEFAULT_AUDIO)).toBe(true);
    expect(audioInRange(withMicrophone(DEFAULT_AUDIO, { noiseRemoval: { enabled: true, strength: 101 } }))).toBe(false);
  });
});

describe("meterPercent", () => {
  it("maps the floor to empty, 0 dB to full and clamps outside", () => {
    expect(meterPercent(-60)).toBe(0);
    expect(meterPercent(-30)).toBe(50);
    expect(meterPercent(0)).toBe(100);
    expect(meterPercent(-100)).toBe(0);
    expect(meterPercent(6)).toBe(100);
    expect(meterPercent(Number.NaN)).toBe(0);
  });
});

describe("reduceAudio", () => {
  const running = { state: "running" } as const;

  it("keeps the levels of each path and the status", () => {
    let state = reduceAudio(initialAudioState, { kind: "audioStatus", microphone: running, speakers: { state: "off" } });
    state = reduceAudio(state, { kind: "audioLevels", path: "microphone", inputDb: -30, outputDb: -20 });
    expect(state.microphone).toEqual(running);
    expect(state.levels.microphone).toEqual({ inputDb: -30, outputDb: -20 });
    expect(state.levels.speakers.inputDb).toBe(-100);
  });

  it("empties the meter of a path that stops", () => {
    let state = reduceAudio(initialAudioState, { kind: "audioStatus", microphone: running, speakers: running });
    state = reduceAudio(state, { kind: "audioLevels", path: "speakers", inputDb: -10, outputDb: -10 });
    state = reduceAudio(state, { kind: "audioStatus", microphone: running, speakers: { state: "unavailable", reason: "gone" } });
    expect(state.levels.speakers.outputDb).toBe(-100);
    expect(state.speakers).toEqual({ state: "unavailable", reason: "gone" });
  });

  it("ignores events that are not about audio", () => {
    expect(reduceAudio(initialAudioState, { kind: "previewDropped", total: 3 })).toBe(initialAudioState);
  });
});
