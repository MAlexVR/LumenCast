import type { MessageKey } from "../i18n";
import type { ErrorPayload } from "../types";

const MESSAGES = new Map<string, MessageKey>([
  ["device_not_found", "err_device_not_found"],
  ["device_busy", "err_device_busy"],
  ["permission_denied", "err_permission_denied"],
  ["mode_unsupported", "err_mode_unsupported"],
  ["mode_invalid", "err_mode_invalid"],
  ["acquisition_failed", "err_acquisition_failed"],
  ["decode_failed", "err_decode_failed"],
  ["cancelled", "err_cancelled"],
  ["capture_already_running", "err_capture_already_running"],
  ["capture_not_running", "err_capture_not_running"],
  ["invalid_setting", "err_invalid_setting"],
  ["format_not_previewable", "err_format_not_previewable"],
  ["settings_not_saved", "err_settings_not_saved"],
  ["autostart_failed", "err_autostart_failed"],
  ["audio_unavailable", "err_audio_unavailable"],
  ["profile_file", "err_profile_file"],
  ["update_not_found", "err_update_not_found"],
  ["update_refused", "err_update_refused"],
  ["update_cancelled", "err_update_cancelled"],
  ["update_network", "err_update_network"],
  ["update_unverified", "err_update_unverified"],
  ["update_not_authorized", "err_update_not_authorized"],
  ["update_unmanaged", "err_update_unmanaged"],
  ["update_install_failed", "err_update_install_failed"],
  ["update_nothing_newer", "err_update_nothing_newer"],
  ["update_no_installer", "err_update_no_installer"],
  ["update_failed", "err_update_failed"],
]);

/** Codes the Rust side can send; keep in sync with `CaptureError::code` and `AppError::code`. */
export const KNOWN_ERROR_CODES: readonly string[] = [...MESSAGES.keys()];

/** Tauri rejects with the serialized `{code, message}`; anything else becomes a generic error. */
export function toErrorPayload(error: unknown): ErrorPayload {
  if (typeof error === "object" && error !== null) {
    const { code, message } = error as Record<string, unknown>;
    if (typeof code === "string" && typeof message === "string") return { code, message };
  }
  const message = error instanceof Error ? error.message : String(error);
  return { code: "unknown", message };
}

export function errorMessageKey(code: string): MessageKey {
  return MESSAGES.get(code) ?? "err_unknown";
}
