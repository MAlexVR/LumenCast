import { describe, expect, it } from "vitest";
import type { CaptureMode } from "../types";
import type { CameraInfo } from "../types";
import {
  fpsLabel,
  initialSelection,
  intervalFps,
  optionsFor,
  pickCamera,
  preferredSelection,
  previewRatio,
  reconcile,
  sizeLabel,
} from "./modes";

const mode = (format: string, width: number, height: number, den: number, num = 1): CaptureMode => ({
  format,
  size: { width, height },
  interval: { numerator: num, denominator: den },
});

const modes: CaptureMode[] = [
  mode("YUYV", 640, 480, 30),
  mode("MJPG", 640, 480, 30),
  mode("MJPG", 1280, 720, 30),
  mode("MJPG", 1280, 720, 15),
  mode("MJPG", 2560, 1920, 15),
  mode("YUYV", 1920, 1080, 30),
];

const has = (selection: { format: string; size: CaptureMode["size"]; interval: CaptureMode["interval"] }) =>
  modes.some(
    (m) =>
      m.format === selection.format &&
      m.size.width === selection.size.width &&
      m.size.height === selection.size.height &&
      m.interval.numerator === selection.interval.numerator &&
      m.interval.denominator === selection.interval.denominator,
  );

describe("labels", () => {
  it("formats frame rates without inventing precision", () => {
    expect(intervalFps({ numerator: 1, denominator: 30 })).toBe(30);
    expect(fpsLabel({ numerator: 1, denominator: 30 })).toBe("30 fps");
    expect(fpsLabel({ numerator: 1001, denominator: 30000 })).toBe("29.97 fps");
    expect(fpsLabel({ numerator: 2, denominator: 15 })).toBe("7.5 fps");
    expect(sizeLabel({ width: 1280, height: 720 })).toBe("1280×720");
  });
});

describe("previewRatio", () => {
  it("follows the real resolution so the preview box is never cropped or stretched", () => {
    expect(previewRatio({ width: 1280, height: 720 })).toBeCloseTo(16 / 9, 6);
    expect(previewRatio({ width: 640, height: 480 })).toBeCloseTo(4 / 3, 6);
    expect(previewRatio({ width: 2560, height: 1920 })).toBeCloseTo(4 / 3, 6);
    expect(previewRatio({ width: 1440, height: 1080 })).toBeCloseTo(4 / 3, 6);
  });

  it("falls back to 16:9 when no size is known or it is degenerate", () => {
    expect(previewRatio(null)).toBeCloseTo(16 / 9, 6);
    expect(previewRatio({ width: 0, height: 720 })).toBeCloseTo(16 / 9, 6);
    expect(previewRatio({ width: 1280, height: 0 })).toBeCloseTo(16 / 9, 6);
  });
});

describe("optionsFor", () => {
  it("lists formats, then only sizes of that format, then only its intervals", () => {
    expect(optionsFor(modes, {}).formats).toEqual(["MJPG", "YUYV"]);
    expect(optionsFor(modes, { format: "MJPG" }).sizes.map(sizeLabel)).toEqual(["640×480", "1280×720", "2560×1920"]);
    const intervals = optionsFor(modes, { format: "MJPG", size: { width: 1280, height: 720 } }).intervals;
    expect(intervals.map(fpsLabel)).toEqual(["15 fps", "30 fps"]);
  });

  it("does not offer an interval that exists only for another format or size", () => {
    const intervals = optionsFor(modes, { format: "MJPG", size: { width: 2560, height: 1920 } }).intervals;
    expect(intervals.map(fpsLabel)).toEqual(["15 fps"]);
  });
});

describe("initialSelection", () => {
  it("prefers MJPG 1280x720 at 30 fps when the camera has it", () => {
    expect(initialSelection(modes)).toEqual({
      format: "MJPG",
      size: { width: 1280, height: 720 },
      interval: { numerator: 1, denominator: 30 },
    });
  });

  it("otherwise takes a real mode and is null for an empty list", () => {
    const only = [mode("YUYV", 320, 240, 30)];
    expect(initialSelection(only)?.format).toBe("YUYV");
    expect(initialSelection([])).toBeNull();
  });
});

describe("reconcile", () => {
  const start = initialSelection(modes)!;

  it("keeps the size when the new format has it and snaps to the nearest otherwise", () => {
    expect(reconcile(modes, start, { format: "YUYV" }).size).toEqual({ width: 1920, height: 1080 });
    const small = reconcile(modes, { ...start, size: { width: 640, height: 480 } }, { format: "YUYV" });
    expect(small.size).toEqual({ width: 640, height: 480 });
  });

  it("snaps the interval to the nearest supported one for the new size", () => {
    const next = reconcile(modes, start, { size: { width: 2560, height: 1920 } });
    expect(next.interval).toEqual({ numerator: 1, denominator: 15 });
  });

  it("never returns a combination the camera did not advertise", () => {
    for (const from of modes) {
      for (const to of modes) {
        const base = { format: from.format, size: from.size, interval: from.interval };
        expect(has(reconcile(modes, base, { format: to.format }))).toBe(true);
        expect(has(reconcile(modes, base, { size: to.size }))).toBe(true);
        expect(has(reconcile(modes, base, { interval: to.interval }))).toBe(true);
      }
    }
  });
});

describe("preferredSelection", () => {
  const yuyv = mode("YUYV", 640, 480, 30);

  it("uses the exact mode of the last capture when the camera still offers it", () => {
    expect(preferredSelection(modes, yuyv, null)).toEqual({ format: "YUYV", size: yuyv.size, interval: yuyv.interval });
  });

  it("falls back to the legacy size and fps, preferring MJPG over other formats", () => {
    const hint = { width: 640, height: 480, fps: 30 };
    expect(preferredSelection(modes, null, hint)?.format).toBe("MJPG");
    const onlyYuyv = modes.filter((m) => m.format === "YUYV");
    expect(preferredSelection(onlyYuyv, null, hint)?.format).toBe("YUYV");
  });

  it("ignores a remembered mode the camera no longer offers and uses the default", () => {
    const gone = mode("MJPG", 1920, 1080, 60);
    expect(preferredSelection(modes, gone, { width: 4000, height: 3000, fps: 5 })).toEqual(initialSelection(modes));
    expect(preferredSelection(modes, null, null)).toEqual(initialSelection(modes));
    expect(preferredSelection([], gone, null)).toBeNull();
  });

  it("prefers the exact mode over the hint", () => {
    expect(preferredSelection(modes, yuyv, { width: 1280, height: 720, fps: 30 })?.format).toBe("YUYV");
  });
});

describe("pickCamera", () => {
  const camera = (index: number, usbId: string | null, name: string | null): CameraInfo => ({
    path: `/dev/video${index}`,
    index,
    name,
    usbId,
  });
  const cameras = [camera(0, "5986:115f", "Integrated Camera"), camera(2, "04e8:205d", "SAMSUNG VD USB HDCAM")];

  it("keeps the current camera while it is still listed", () => {
    expect(pickCamera(cameras, { usbId: "04e8:205d", name: null }, 0)?.index).toBe(0);
  });

  it("finds the remembered camera by USB id even if its number changed", () => {
    const renumbered = [camera(0, "5986:115f", "Integrated Camera"), camera(4, "04e8:205d", "SAMSUNG VD USB HDCAM")];
    expect(pickCamera(renumbered, { usbId: "04e8:205d", name: "x" }, null)?.index).toBe(4);
  });

  it("falls back to the name, then to the first camera, and is undefined for none", () => {
    expect(pickCamera(cameras, { usbId: null, name: "SAMSUNG VD USB HDCAM" }, null)?.index).toBe(2);
    expect(pickCamera(cameras, { usbId: "dead:beef", name: "gone" }, null)?.index).toBe(0);
    expect(pickCamera(cameras, null, 9)?.index).toBe(0);
    expect(pickCamera([], null, null)).toBeUndefined();
  });
});
