import type { MessageKey } from "../i18n";
import { fpsLabel, sizeLabel } from "./modes";
import type { EndReason, ModeField, NegotiationReport, StatsSnapshot } from "../types";

export interface NegotiationRow {
  field: ModeField;
  labelKey: MessageKey;
  requested: string;
  negotiated: string;
  differs: boolean;
}

export function negotiationRows(report: NegotiationReport): NegotiationRow[] {
  const { requested, negotiated, differences } = report;
  const row = (field: ModeField, labelKey: MessageKey, asked: string, granted: string): NegotiationRow => ({
    field,
    labelKey,
    requested: asked,
    negotiated: granted,
    differs: differences.includes(field),
  });
  return [
    row("format", "format", requested.format, negotiated.format),
    row("size", "size", sizeLabel(requested.size), sizeLabel(negotiated.size)),
    row("interval", "frame_rate", fpsLabel(requested.interval), fpsLabel(negotiated.interval)),
  ];
}

export function fpsText(stats: StatsSnapshot | null): string {
  return stats?.measuredFps == null ? "—" : stats.measuredFps.toFixed(1);
}

/** Frames handed to the preview minus the ones the backend then dropped. */
export function previewShown(stats: StatsSnapshot, dropped: number): number {
  return Math.max(0, stats.framesPreviewed - dropped);
}

/** Enough frames were dropped and almost none shown: the picture on screen, if any, is stale. */
export function previewStalled(stats: StatsSnapshot, dropped: number): boolean {
  return dropped >= 30 && previewShown(stats, dropped) * 10 < dropped;
}

export function endMessageKey(end: EndReason): MessageKey {
  switch (end.kind) {
    case "cancelled":
      return "ended_cancelled";
    case "frameLimit":
      return "ended_limit";
    case "failed":
      return "ended_failed";
  }
}
