import { describe, expect, it } from "vitest";
import type { CaptureMode, NegotiationReport, StatsSnapshot } from "../types";
import { endMessageKey, fpsText, negotiationRows, previewShown, previewStalled } from "./view";

const mode = (format: string, width: number, height: number, den: number): CaptureMode => ({
  format,
  size: { width, height },
  interval: { numerator: 1, denominator: den },
});

describe("negotiationRows", () => {
  it("shows requested next to negotiated and flags only the fields that differ", () => {
    const report: NegotiationReport = {
      requested: mode("MJPG", 1920, 1080, 60),
      negotiated: mode("MJPG", 1280, 720, 30),
      differences: ["size", "interval"],
    };
    expect(negotiationRows(report)).toEqual([
      { field: "format", labelKey: "format", requested: "MJPG", negotiated: "MJPG", differs: false },
      { field: "size", labelKey: "size", requested: "1920×1080", negotiated: "1280×720", differs: true },
      { field: "interval", labelKey: "frame_rate", requested: "60 fps", negotiated: "30 fps", differs: true },
    ]);
  });
});

describe("fpsText", () => {
  const stats = (measuredFps: number | null): StatsSnapshot => ({
    framesCaptured: 0,
    framesPreviewed: 0,
    acquisitionErrors: 0,
    decodeErrors: 0,
    outOfOrderTimestamps: 0,
    measuredFps,
  });

  it("shows a dash until fps is measured, then one decimal", () => {
    expect(fpsText(null)).toBe("—");
    expect(fpsText(stats(null))).toBe("—");
    expect(fpsText(stats(30.05928752415794))).toBe("30.1");
  });
});

describe("previewShown", () => {
  const stats = (framesPreviewed: number): StatsSnapshot => ({
    framesCaptured: 100,
    framesPreviewed,
    acquisitionErrors: 0,
    decodeErrors: 0,
    outOfOrderTimestamps: 0,
    measuredFps: 30,
  });

  it("is the frames handed to the preview minus the ones that were dropped", () => {
    expect(previewShown(stats(20), 0)).toBe(20);
    expect(previewShown(stats(20), 5)).toBe(15);
  });

  it("never goes below zero while counters from different events catch up", () => {
    expect(previewShown(stats(2), 5)).toBe(0);
  });
});

describe("endMessageKey", () => {
  it("has a distinct message per end reason", () => {
    expect(endMessageKey({ kind: "cancelled" })).toBe("ended_cancelled");
    expect(endMessageKey({ kind: "frameLimit" })).toBe("ended_limit");
    expect(endMessageKey({ kind: "failed", error: { code: "x", message: "y" } })).toBe("ended_failed");
  });
});

describe("previewStalled", () => {
  const stats = (framesPreviewed: number): StatsSnapshot => ({
    framesCaptured: 120,
    framesPreviewed,
    acquisitionErrors: 0,
    decodeErrors: 0,
    outOfOrderTimestamps: 0,
    measuredFps: 30,
  });

  it("is false while few frames were dropped or most were shown", () => {
    expect(previewStalled(stats(120), 10)).toBe(false);
    expect(previewStalled(stats(120), 40)).toBe(false);
  });

  it("is true when many were dropped and almost none got through", () => {
    expect(previewStalled(stats(120), 119)).toBe(true);
  });
});
