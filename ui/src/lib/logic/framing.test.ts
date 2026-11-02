import { describe, expect, it } from "vitest";
import { DEFAULT_PROCESSING, PAN_STEP, autoStatusKey, canPan, centerPan, isNeutral, nudgePan, withAutoFraming, withBackground, withDenoise, withKeyLight, withZoom, zoomFromLevel, zoomLevel } from "./framing";

const zoomed = { ...DEFAULT_PROCESSING, zoomPercent: 150 };

describe("nudgePan", () => {
  it("moves one legacy step in the pressed direction", () => {
    expect(PAN_STEP).toBe(0.1);
    expect(nudgePan(zoomed, "right").panX).toBe(0.1);
    expect(nudgePan(zoomed, "left").panX).toBe(-0.1);
    expect(nudgePan(zoomed, "down").panY).toBe(0.1);
    expect(nudgePan(zoomed, "up").panY).toBe(-0.1);
  });

  it("does not accumulate floating point drift and stops at the edges", () => {
    let state = zoomed;
    for (let i = 0; i < 10; i++) state = nudgePan(state, "right");
    expect(state.panX).toBe(1);
    expect(nudgePan(state, "right").panX).toBe(1);
    for (let i = 0; i < 25; i++) state = nudgePan(state, "left");
    expect(state.panX).toBe(-1);
  });

  it("leaves every other setting alone", () => {
    const base = { ...zoomed, mirrorPreview: true, vignette: 0.4 };
    expect(nudgePan(base, "up")).toEqual({ ...base, panY: -0.1 });
  });
});

describe("centerPan and withZoom", () => {
  it("centers both axes", () => {
    expect(centerPan({ ...zoomed, panX: 0.7, panY: -0.3 })).toEqual(zoomed);
  });

  it("rounds to whole percent and keeps the legacy 100 to 200 range", () => {
    expect(withZoom(DEFAULT_PROCESSING, 149.6).zoomPercent).toBe(150);
    expect(withZoom(DEFAULT_PROCESSING, 50).zoomPercent).toBe(100);
    expect(withZoom(DEFAULT_PROCESSING, 999).zoomPercent).toBe(200);
    expect(withZoom(DEFAULT_PROCESSING, Number.NaN).zoomPercent).toBe(100);
  });
});

describe("canPan and isNeutral", () => {
  it("allows panning only when zoomed", () => {
    expect(canPan(DEFAULT_PROCESSING)).toBe(false);
    expect(canPan(zoomed)).toBe(true);
  });

  it("is neutral only when nothing would change the picture", () => {
    expect(isNeutral(DEFAULT_PROCESSING)).toBe(true);
    expect(isNeutral({ ...DEFAULT_PROCESSING, panX: 0.9 })).toBe(true);
    expect(isNeutral({ ...DEFAULT_PROCESSING, mirrorPreview: true })).toBe(false);
    expect(isNeutral({ ...DEFAULT_PROCESSING, vignette: 0.1 })).toBe(false);
    expect(isNeutral(zoomed)).toBe(false);
    expect(isNeutral(withAutoFraming(DEFAULT_PROCESSING, { enabled: true }))).toBe(false);
  });
});

describe("withAutoFraming", () => {
  it("changes only the given auto framing fields and keeps the rest", () => {
    const next = withAutoFraming({ ...DEFAULT_PROCESSING, mirrorPreview: true }, { speed: 80 });
    expect(next.autoFraming).toEqual({ enabled: false, minZoomPercent: 130, speed: 80 });
    expect(next.mirrorPreview).toBe(true);
  });

  it("keeps the values inside the legacy ranges and rounds them", () => {
    expect(withAutoFraming(DEFAULT_PROCESSING, { minZoomPercent: 50 }).autoFraming.minZoomPercent).toBe(100);
    expect(withAutoFraming(DEFAULT_PROCESSING, { minZoomPercent: 250 }).autoFraming.minZoomPercent).toBe(200);
    expect(withAutoFraming(DEFAULT_PROCESSING, { minZoomPercent: 134.6 }).autoFraming.minZoomPercent).toBe(135);
    expect(withAutoFraming(DEFAULT_PROCESSING, { speed: 140 }).autoFraming.speed).toBe(100);
    expect(withAutoFraming(DEFAULT_PROCESSING, { speed: -5 }).autoFraming.speed).toBe(0);
    expect(withAutoFraming(DEFAULT_PROCESSING, { speed: Number.NaN }).autoFraming.speed).toBe(35);
  });
});

describe("autoStatusKey", () => {
  it("has a message for each state that needs one", () => {
    expect(autoStatusKey(null)).toBeNull();
    expect(autoStatusKey({ state: "off" })).toBeNull();
    expect(autoStatusKey({ state: "searching" })).toBe("auto_searching");
    expect(autoStatusKey({ state: "unavailable" })).toBe("auto_unavailable");
    expect(autoStatusKey({ state: "locked", zoomPercent: 140, panX: 0, panY: 0, reachable: true })).toBe("auto_locked");
    // Being out of reach is not something to ask the user about: the crop just stays at its edge.
    expect(autoStatusKey({ state: "locked", zoomPercent: 140, panX: 1, panY: 0, reachable: false })).toBe("auto_locked");
  });
});

describe("withKeyLight", () => {
  it("changes only what the patch names and keeps levels in range", () => {
    const on = withKeyLight(DEFAULT_PROCESSING, { enabled: true });
    expect(on.keyLight).toEqual({ enabled: true, intensity: 40, warmth: 60, lightPicture: null });
    expect(withKeyLight(on, { lightPicture: "/x/studio.png" }).keyLight.lightPicture).toBe("/x/studio.png");
    expect(withKeyLight(withKeyLight(on, { lightPicture: "/x/studio.png" }), { intensity: 10 }).keyLight.lightPicture).toBe("/x/studio.png");
    expect(withKeyLight(withKeyLight(on, { lightPicture: "/x/studio.png" }), { lightPicture: null }).keyLight.lightPicture).toBeNull();
    expect(withKeyLight(on, { intensity: 250 }).keyLight.intensity).toBe(100);
    expect(withKeyLight(on, { warmth: -5 }).keyLight.warmth).toBe(0);
    expect(withKeyLight(on, { warmth: Number.NaN }).keyLight.warmth).toBe(60);
  });

  it("makes the settings non neutral while the light is on", () => {
    expect(isNeutral(DEFAULT_PROCESSING)).toBe(true);
    expect(isNeutral(withKeyLight(DEFAULT_PROCESSING, { enabled: true }))).toBe(false);
  });
});

describe("withBackground", () => {
  it("changes only what the patch names and keeps the blur in range", () => {
    const blurred = withBackground(DEFAULT_PROCESSING, { mode: "blur" });
    expect(blurred.background).toEqual({ engine: "nvidia", mode: "blur", blurStrength: 50, imagePath: null });
    expect(withBackground(blurred, { engine: "builtin" }).background.engine).toBe("builtin");
    expect(withBackground(blurred, { blurStrength: 400 }).background.blurStrength).toBe(100);
    expect(withBackground(blurred, { blurStrength: Number.NaN }).background.blurStrength).toBe(50);
    expect(withBackground(blurred, { imagePath: "/a/b.png" }).background.imagePath).toBe("/a/b.png");
    expect(withBackground(withBackground(blurred, { imagePath: "/a/b.png" }), { imagePath: null }).background.imagePath).toBeNull();
  });

  it("makes the settings non neutral while a background mode is chosen", () => {
    expect(isNeutral(withBackground(DEFAULT_PROCESSING, { mode: "remove" }))).toBe(false);
  });
});

describe("withDenoise", () => {
  it("changes only what the patch names and keeps the strength in range", () => {
    const on = withDenoise(DEFAULT_PROCESSING, { enabled: true });
    expect(on.denoise).toEqual({ enabled: true, strength: 60 });
    expect(withDenoise(on, { strength: -3 }).denoise.strength).toBe(0);
    expect(withDenoise(on, { strength: 999 }).denoise.strength).toBe(100);
    expect(isNeutral(on)).toBe(false);
  });
});

describe("zoom levels", () => {
  it("map the backend 100 to 200 range onto 0 to 100", () => {
    expect(zoomLevel(100)).toBe(0);
    expect(zoomLevel(150)).toBe(50);
    expect(zoomLevel(200)).toBe(100);
    expect(zoomLevel(126.4)).toBe(26);
    expect(zoomLevel(80)).toBe(0);
    expect(zoomLevel(400)).toBe(100);
  });

  it("and back, clamped", () => {
    expect(zoomFromLevel(0)).toBe(100);
    expect(zoomFromLevel(100)).toBe(200);
    expect(zoomFromLevel(-5)).toBe(100);
    expect(zoomFromLevel(300)).toBe(200);
    expect(zoomFromLevel(Number.NaN)).toBe(100);
    for (const level of [0, 13, 57, 100]) expect(zoomLevel(zoomFromLevel(level))).toBe(level);
  });
});
