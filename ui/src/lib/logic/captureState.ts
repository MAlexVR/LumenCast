import type { AppEvent, AutoFramingStatus, BackgroundStatus, VirtualCameraStatus, ErrorPayload, NegotiationReport, SessionOutcome, StatsSnapshot } from "../types";

export type Phase = "idle" | "starting" | "running" | "ended";

export interface CaptureState {
  phase: Phase;
  report: NegotiationReport | null;
  stats: StatsSnapshot | null;
  previewBase64: string | null;
  outcome: SessionOutcome | null;
  error: ErrorPayload | null;
  /** Preview frames the backend could not decode and dropped. */
  previewDropped: number;
  autoFraming: AutoFramingStatus | null;
  background: BackgroundStatus | null;
  virtualCamera: VirtualCameraStatus | null;
}

export type Action =
  | { type: "start" }
  | { type: "rejected"; error: ErrorPayload }
  | { type: "event"; event: AppEvent };

export const initialState: CaptureState = {
  phase: "idle",
  report: null,
  stats: null,
  previewBase64: null,
  outcome: null,
  error: null,
  previewDropped: 0,
  autoFraming: null,
  background: null,
  virtualCamera: null,
};

export function reduce(state: CaptureState, action: Action): CaptureState {
  switch (action.type) {
    case "start":
      return { ...initialState, phase: "starting" };
    case "rejected":
      return { ...state, phase: "idle", error: action.error };
    case "event":
      return applyEvent(state, action.event);
  }
}

function applyEvent(state: CaptureState, event: AppEvent): CaptureState {
  // Events that outlive their session (for example after a rejected start) are stale.
  if (state.phase === "idle") return state;
  switch (event.kind) {
    case "negotiated":
      return { ...state, phase: "running", report: event.report, error: null };
    case "frame":
      return { ...state, previewBase64: event.jpegBase64 };
    case "stats":
      return { ...state, stats: event.stats };
    case "previewDropped":
      // Two threads report this counter; the larger total is the later one.
      return { ...state, previewDropped: Math.max(state.previewDropped, event.total) };
    case "autoFraming":
      return { ...state, autoFraming: event.status };
    case "background":
      return { ...state, background: event.status };
    case "virtualCamera":
      return { ...state, virtualCamera: event.status };
    case "audioStatus":
    case "audioLevels":
      // Audio has its own state, which does not depend on a capture session.
      return state;
    case "ended":
      return event.outcome
        ? { ...state, phase: "ended", outcome: event.outcome, stats: event.outcome.stats, report: event.outcome.report, error: null }
        : { ...state, phase: "ended", outcome: null, error: event.error };
  }
}
