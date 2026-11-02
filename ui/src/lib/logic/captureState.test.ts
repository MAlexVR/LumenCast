import { describe, expect, it } from "vitest";
import type { AppEvent, NegotiationReport, SessionOutcome, StatsSnapshot } from "../types";
import { initialState, reduce, type Action, type CaptureState } from "./captureState";

const mode = { format: "MJPG", size: { width: 1280, height: 720 }, interval: { numerator: 1, denominator: 30 } };
const report: NegotiationReport = { requested: mode, negotiated: mode, differences: [] };
const stats: StatsSnapshot = {
  framesCaptured: 5,
  framesPreviewed: 2,
  acquisitionErrors: 0,
  decodeErrors: 1,
  outOfOrderTimestamps: 0,
  measuredFps: 30.1,
};
const outcome: SessionOutcome = { report, stats, end: { kind: "frameLimit" } };
const apply = (state: CaptureState, ...actions: Action[]) => actions.reduce(reduce, state);
const event = (e: AppEvent): Action => ({ type: "event", event: e });

describe("capture state", () => {
  it("moves idle -> starting -> running when the negotiated mode arrives", () => {
    const starting = apply(initialState, { type: "start" });
    expect(starting.phase).toBe("starting");
    const running = apply(starting, event({ kind: "negotiated", report }));
    expect(running.phase).toBe("running");
    expect(running.report).toEqual(report);
  });

  it("keeps the latest preview frame and stats while running", () => {
    const state = apply(
      initialState,
      { type: "start" },
      event({ kind: "negotiated", report }),
      event({ kind: "frame", timestampUs: 1, jpegBase64: "AAA" }),
      event({ kind: "frame", timestampUs: 2, jpegBase64: "BBB" }),
      event({ kind: "stats", stats }),
    );
    expect(state.previewBase64).toBe("BBB");
    expect(state.stats).toEqual(stats);
  });

  it("ends with the outcome and keeps the last frame visible", () => {
    const state = apply(
      initialState,
      { type: "start" },
      event({ kind: "negotiated", report }),
      event({ kind: "frame", timestampUs: 1, jpegBase64: "AAA" }),
      event({ kind: "ended", outcome, error: null }),
    );
    expect(state.phase).toBe("ended");
    expect(state.outcome).toEqual(outcome);
    expect(state.stats).toEqual(stats);
    expect(state.previewBase64).toBe("AAA");
  });

  it("ends with an error when the session never started", () => {
    const error = { code: "mode_unsupported", message: "no" };
    const state = apply(initialState, { type: "start" }, event({ kind: "ended", outcome: null, error }));
    expect(state.phase).toBe("ended");
    expect(state.error).toEqual(error);
    expect(state.outcome).toBeNull();
  });

  it("returns to idle with the error when a command is rejected", () => {
    const error = { code: "capture_already_running", message: "busy" };
    const state = apply(initialState, { type: "start" }, { type: "rejected", error });
    expect(state.phase).toBe("idle");
    expect(state.error).toEqual(error);
  });

  it("ignores stale frame and stats events that arrive while idle", () => {
    const state = apply(initialState, event({ kind: "frame", timestampUs: 1, jpegBase64: "ZZZ" }), event({ kind: "stats", stats }));
    expect(state).toEqual(initialState);
  });

  it("counts the preview frames the backend dropped and resets them for a new session", () => {
    const running = apply(
      initialState,
      { type: "start" },
      event({ kind: "negotiated", report }),
      event({ kind: "previewDropped", total: 1 }),
      event({ kind: "previewDropped", total: 2 }),
    );
    expect(running.previewDropped).toBe(2);
    expect(apply(running, { type: "start" }).previewDropped).toBe(0);
  });

  it("keeps the largest dropped total when two threads report out of order", () => {
    const state = apply(
      initialState,
      { type: "start" },
      event({ kind: "negotiated", report }),
      event({ kind: "previewDropped", total: 5 }),
      event({ kind: "previewDropped", total: 4 }),
    );
    expect(state.previewDropped).toBe(5);
  });

  it("keeps the latest auto framing state and forgets it when a new capture starts", () => {
    const locked = { state: "locked", zoomPercent: 140, panX: 0.2, panY: 0, reachable: true } as const;
    const running = apply(
      initialState,
      { type: "start" },
      event({ kind: "negotiated", report }),
      event({ kind: "autoFraming", status: { state: "searching" } }),
      event({ kind: "autoFraming", status: locked }),
    );
    expect(running.autoFraming).toEqual(locked);
    expect(apply(running, { type: "start" }).autoFraming).toBeNull();
  });

  it("clears the previous session when a new one starts", () => {
    const ended = apply(initialState, { type: "start" }, event({ kind: "ended", outcome, error: null }));
    const next = apply(ended, { type: "start" });
    expect(next).toEqual({ ...initialState, phase: "starting" });
  });
});

describe("virtual camera status", () => {
  it("is kept from the events of the session and cleared by a new one", () => {
    const active = { state: "active", device: "/dev/video10", width: 1280, height: 720 } as const;
    const running = apply(
      initialState,
      { type: "start" },
      event({ kind: "negotiated", report }),
      event({ kind: "virtualCamera", status: active }),
    );
    expect(running.virtualCamera).toEqual(active);
    expect(apply(running, { type: "start" }).virtualCamera).toBeNull();
  });
});
