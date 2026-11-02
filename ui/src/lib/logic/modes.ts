import type { CameraInfo, CaptureMode, FrameInterval, FrameSize, ModeHint, PreferredCamera } from "../types";

export interface Selection {
  format: string;
  size: FrameSize;
  interval: FrameInterval;
}

const sameSize = (a: FrameSize, b: FrameSize) => a.width === b.width && a.height === b.height;
const sameInterval = (a: FrameInterval, b: FrameInterval) => a.numerator * b.denominator === b.numerator * a.denominator;
const area = (size: FrameSize) => size.width * size.height;

export function intervalFps(interval: FrameInterval): number {
  return interval.denominator / interval.numerator;
}

export function fpsLabel(interval: FrameInterval): string {
  const fps = intervalFps(interval);
  const text = Number.isInteger(fps) ? String(fps) : String(Math.round(fps * 100) / 100);
  return `${text} fps`;
}

/** Width over height of the preview box; 16:9 until a size is known. */
export function previewRatio(size: FrameSize | null): number {
  return size && size.width > 0 && size.height > 0 ? size.width / size.height : 16 / 9;
}

export function sizeLabel(size: FrameSize): string {
  return `${size.width}×${size.height}`;
}

function unique<T>(items: T[], same: (a: T, b: T) => boolean): T[] {
  return items.filter((item, index) => items.findIndex((other) => same(item, other)) === index);
}

export function optionsFor(
  modes: readonly CaptureMode[],
  partial: { format?: string; size?: FrameSize },
): { formats: string[]; sizes: FrameSize[]; intervals: FrameInterval[] } {
  const formats = [...new Set(modes.map((mode) => mode.format))].sort();
  const ofFormat = modes.filter((mode) => partial.format === undefined || mode.format === partial.format);
  const sizes = unique(
    ofFormat.map((mode) => mode.size),
    sameSize,
  ).sort((a, b) => area(a) - area(b) || a.width - b.width);
  const ofSize = ofFormat.filter((mode) => partial.size === undefined || sameSize(mode.size, partial.size));
  const intervals = unique(
    ofSize.map((mode) => mode.interval),
    sameInterval,
  ).sort((a, b) => intervalFps(a) - intervalFps(b));
  return { formats, sizes, intervals };
}

/** The UI's first choice. It is always one of `modes`, never a fallback outside them. */
export function initialSelection(modes: readonly CaptureMode[]): Selection | null {
  const preferred = modes.find(
    (mode) =>
      mode.format === "MJPG" && mode.size.width === 1280 && mode.size.height === 720 && intervalFps(mode.interval) === 30,
  );
  const mode = preferred ?? modes[0];
  return mode ? { format: mode.format, size: mode.size, interval: mode.interval } : null;
}

/**
 * The mode to start from: the exact mode of the last capture when the camera
 * still offers it, else the legacy size and frame rate (preferring MJPG),
 * else the usual default. Always one of `modes`.
 */
export function preferredSelection(
  modes: readonly CaptureMode[],
  preferred: CaptureMode | null,
  hint: ModeHint | null,
): Selection | null {
  const asSelection = (mode: CaptureMode): Selection => ({ format: mode.format, size: mode.size, interval: mode.interval });
  if (preferred) {
    const exact = modes.find(
      (mode) => mode.format === preferred.format && sameSize(mode.size, preferred.size) && sameInterval(mode.interval, preferred.interval),
    );
    if (exact) return asSelection(exact);
  }
  if (hint) {
    const matching = modes.filter(
      (mode) =>
        mode.size.width === hint.width && mode.size.height === hint.height && Math.abs(intervalFps(mode.interval) - hint.fps) < 0.01,
    );
    const best = matching.find((mode) => mode.format === "MJPG") ?? matching[0];
    if (best) return asSelection(best);
  }
  return initialSelection(modes);
}

/** Keeps the current camera if it is still listed, else the preferred one, else the first. */
export function pickCamera(
  cameras: readonly CameraInfo[],
  preferred: PreferredCamera | null,
  currentIndex: number | null,
): CameraInfo | undefined {
  const current = currentIndex === null ? undefined : cameras.find((camera) => camera.index === currentIndex);
  if (current) return current;
  if (preferred) {
    const byUsb = preferred.usbId ? cameras.find((camera) => camera.usbId === preferred.usbId) : undefined;
    const byName = preferred.name ? cameras.find((camera) => camera.name === preferred.name) : undefined;
    if (byUsb ?? byName) return byUsb ?? byName;
  }
  return cameras[0];
}

function nearestSize(sizes: FrameSize[], wanted: FrameSize): FrameSize {
  const exact = sizes.find((size) => sameSize(size, wanted));
  if (exact) return exact;
  // Compare by pixel-count ratio so "closer" does not depend on absolute scale.
  const distance = (size: FrameSize) => Math.abs(Math.log(area(size) / area(wanted)));
  return sizes.reduce((best, size) => (distance(size) < distance(best) ? size : best));
}

function nearestInterval(intervals: FrameInterval[], wanted: FrameInterval): FrameInterval {
  const exact = intervals.find((interval) => sameInterval(interval, wanted));
  if (exact) return exact;
  const distance = (interval: FrameInterval) => Math.abs(intervalFps(interval) - intervalFps(wanted));
  // Ties go to the higher frame rate.
  return intervals.reduce((best, interval) =>
    distance(interval) < distance(best) || (distance(interval) === distance(best) && intervalFps(interval) > intervalFps(best))
      ? interval
      : best,
  );
}

/** Applies one change and returns the nearest selection that really exists. */
export function reconcile(
  modes: readonly CaptureMode[],
  previous: Selection,
  change: { format?: string; size?: FrameSize; interval?: FrameInterval },
): Selection {
  const format = change.format ?? previous.format;
  const size = nearestSize(optionsFor(modes, { format }).sizes, change.size ?? previous.size);
  const interval = nearestInterval(optionsFor(modes, { format, size }).intervals, change.interval ?? previous.interval);
  return { format, size, interval };
}
