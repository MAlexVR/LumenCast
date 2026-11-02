import type { MessageKey } from "../i18n";
import type { InstallKind, UpdatePhase, UpdateProgress } from "../types";

/** Share of the download done (0 to 100), or `null` when it is not downloading or the size is unknown. */
export function progressPercent(progress: UpdateProgress): number | null {
  if (progress.phase !== "downloading" || progress.total <= 0) return null;
  return Math.min(100, Math.max(0, Math.round((progress.done / progress.total) * 100)));
}

/** `48.0 MB`; an unknown size is an empty text. */
export function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  const units = ["B", "KB", "MB", "GB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(1)} ${units[unit]}`;
}

export function phaseKey(phase: UpdatePhase): MessageKey {
  switch (phase) {
    case "downloading":
      return "update_downloading";
    case "verifying":
      return "update_verifying";
    case "installing":
      return "update_installing";
  }
}

export function installKindKey(kind: InstallKind): MessageKey {
  switch (kind) {
    case "rpm":
      return "update_kind_rpm";
    case "deb":
      return "update_kind_deb";
    case "appImage":
      return "update_kind_appimage";
    case "unmanaged":
      return "update_kind_unmanaged";
  }
}
