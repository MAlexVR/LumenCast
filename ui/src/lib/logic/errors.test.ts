import { describe, expect, it } from "vitest";
import { en } from "../i18n/en";
import { es } from "../i18n/es";
import { KNOWN_ERROR_CODES, errorMessageKey, toErrorPayload } from "./errors";

describe("errorMessageKey", () => {
  it("covers every code the backend can send", () => {
    expect([...KNOWN_ERROR_CODES].sort()).toEqual(
      [
        "acquisition_failed",
        "audio_unavailable",
        "update_cancelled",
        "update_failed",
        "update_install_failed",
        "update_network",
        "update_no_installer",
        "update_not_authorized",
        "update_not_found",
        "update_nothing_newer",
        "update_refused",
        "update_unmanaged",
        "update_unverified",
        "cancelled",
        "capture_already_running",
        "capture_not_running",
        "decode_failed",
        "device_busy",
        "device_not_found",
        "format_not_previewable",
        "invalid_setting",
        "mode_invalid",
        "mode_unsupported",
        "permission_denied",
        "settings_not_saved",
        "autostart_failed",
      ].sort(),
    );
  });

  it("maps each known code to its own message in both languages", () => {
    const keys = new Set(KNOWN_ERROR_CODES.map(errorMessageKey));
    expect(keys.size).toBe(KNOWN_ERROR_CODES.length);
    for (const code of KNOWN_ERROR_CODES) {
      const key = errorMessageKey(code);
      expect(es[key].length).toBeGreaterThan(10);
      expect(en[key].length).toBeGreaterThan(10);
    }
  });

  it("falls back to a generic message for unknown codes", () => {
    expect(errorMessageKey("something_new")).toBe("err_unknown");
  });
});

describe("toErrorPayload", () => {
  it("keeps a backend payload as it is", () => {
    const payload = { code: "device_busy", message: "busy" };
    expect(toErrorPayload(payload)).toEqual(payload);
  });

  it("turns anything else into an unknown error that keeps the text", () => {
    expect(toErrorPayload(new Error("boom"))).toEqual({ code: "unknown", message: "boom" });
    expect(toErrorPayload("plain text")).toEqual({ code: "unknown", message: "plain text" });
    expect(toErrorPayload({ code: 5 })).toEqual({ code: "unknown", message: "[object Object]" });
    expect(toErrorPayload(null).code).toBe("unknown");
  });
});
