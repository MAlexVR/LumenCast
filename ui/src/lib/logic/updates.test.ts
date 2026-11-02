import { describe, expect, it } from "vitest";
import { formatSize, installKindKey, phaseKey, progressPercent } from "./updates";

describe("progressPercent", () => {
  it("is the share downloaded, clamped, and null when there is nothing to measure", () => {
    expect(progressPercent({ phase: "downloading", done: 24, total: 48 })).toBe(50);
    expect(progressPercent({ phase: "downloading", done: 99, total: 48 })).toBe(100);
    expect(progressPercent({ phase: "downloading", done: 5, total: 0 })).toBeNull();
    expect(progressPercent({ phase: "verifying", done: 0, total: 0 })).toBeNull();
  });
});

describe("formatSize", () => {
  it("shows decimal megabytes and leaves an unknown size empty", () => {
    expect(formatSize(48_000_000)).toBe("48.0 MB");
    expect(formatSize(1_500)).toBe("1.5 KB");
    expect(formatSize(12)).toBe("12 B");
    expect(formatSize(0)).toBe("");
    expect(formatSize(Number.NaN)).toBe("");
  });
});

describe("keys", () => {
  it("name a message for every phase and install kind", () => {
    expect(phaseKey("downloading")).toBe("update_downloading");
    expect(phaseKey("installing")).toBe("update_installing");
    expect(installKindKey("appImage")).toBe("update_kind_appimage");
    expect(installKindKey("unmanaged")).toBe("update_kind_unmanaged");
  });
});
