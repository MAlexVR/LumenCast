// Typed access to the Tauri commands. Outside Tauri (browser preview,
// headless screenshots) the same interface is served by an in-memory mock.
import { createMockApi } from "./mock/mockApi";
import type { AppEvent, AudioDevice, UpdateProgress, UpdateStatus, AudioPaths, AudioSettings, CameraInfo, CameraProbe, CaptureMode, Preferences, ProcessingSettings } from "./types";

export interface Api {
  listCameras(): Promise<CameraInfo[]>;
  probeCamera(index: number): Promise<CameraProbe>;
  startCapture(index: number, mode: CaptureMode): Promise<void>;
  stopCapture(): Promise<void>;
  getProcessing(): Promise<ProcessingSettings>;
  /** Applies to the next preview frame, also while capturing. */
  setProcessing(settings: ProcessingSettings): Promise<void>;
  getAudio(): Promise<AudioSettings>;
  /** Validates, applies (starting or stopping the audio paths) and saves. */
  setAudio(settings: AudioSettings): Promise<void>;
  getAudioStatus(): Promise<AudioPaths>;
  listAudioDevices(): Promise<AudioDevice[]>;
  /** Saved settings; the import report comes back only the first time. */
  getPreferences(): Promise<Preferences>;
  setLanguage(code: "es" | "en"): Promise<void>;
  /** Opens the system file picker for a PNG or JPEG; `null` when it is cancelled. */
  pickImage(): Promise<string | null>;
  /** Whether LumenCast starts when the user logs in. */
  getAutostart(): Promise<boolean>;
  setAutostart(enabled: boolean): Promise<void>;
  setCloseToTray(enabled: boolean): Promise<void>;
  setCheckUpdates(enabled: boolean): Promise<void>;
  /** Asks GitHub whether a newer version exists. */
  checkUpdate(): Promise<UpdateStatus>;
  /** Downloads, verifies and installs the newest version; the program restarts. */
  installUpdate(): Promise<void>;
  cancelUpdate(): Promise<void>;
  onUpdateProgress(handler: (progress: UpdateProgress) => void): Promise<() => void>;
  /** The tray menu changed a setting the window shows. */
  onTrayChanged(handler: () => void): Promise<() => void>;
  /** The tray menu asked to look for updates. */
  onTrayUpdates(handler: () => void): Promise<() => void>;
  /** Resolves to the function that stops listening. */
  onCaptureEvent(handler: (event: AppEvent) => void): Promise<() => void>;
}

const CAPTURE_EVENT = "capture://event";

function inTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function createTauriApi(): Api {
  const core = import("@tauri-apps/api/core");
  const events = import("@tauri-apps/api/event");
  return {
    listCameras: async () => (await core).invoke<CameraInfo[]>("list_cameras"),
    probeCamera: async (index) => (await core).invoke<CameraProbe>("probe_camera", { index }),
    startCapture: async (index, mode) => (await core).invoke<void>("start_capture", { index, mode }),
    stopCapture: async () => (await core).invoke<void>("stop_capture"),
    getProcessing: async () => (await core).invoke<ProcessingSettings>("get_processing"),
    setProcessing: async (settings) => (await core).invoke<void>("set_processing", { settings }),
    getAudio: async () => (await core).invoke<AudioSettings>("get_audio"),
    setAudio: async (settings) => (await core).invoke<void>("set_audio", { settings }),
    getAudioStatus: async () => (await core).invoke<AudioPaths>("get_audio_status"),
    listAudioDevices: async () => (await core).invoke<AudioDevice[]>("list_audio_devices"),
    getPreferences: async () => (await core).invoke<Preferences>("get_preferences"),
    setLanguage: async (code) => (await core).invoke<void>("set_language", { code }),
    pickImage: async () => {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const chosen = await open({
        multiple: false,
        directory: false,
        filters: [{ name: "PNG / JPEG", extensions: ["png", "jpg", "jpeg"] }],
      });
      return typeof chosen === "string" ? chosen : null;
    },
    getAutostart: async () => (await core).invoke<boolean>("get_autostart"),
    setAutostart: async (enabled) => (await core).invoke<void>("set_autostart", { enabled }),
    setCloseToTray: async (enabled) => (await core).invoke<void>("set_close_to_tray", { enabled }),
    setCheckUpdates: async (enabled) => (await core).invoke<void>("set_check_updates", { enabled }),
    checkUpdate: async () => (await core).invoke<UpdateStatus>("check_update"),
    installUpdate: async () => (await core).invoke<void>("install_update"),
    cancelUpdate: async () => (await core).invoke<void>("cancel_update"),
    onUpdateProgress: async (handler) => (await events).listen<UpdateProgress>("update://progress", (e) => handler(e.payload)),
    onTrayChanged: async (handler) => (await events).listen("tray://changed", () => handler()),
    onTrayUpdates: async (handler) => (await events).listen("tray://updates", () => handler()),
    onCaptureEvent: async (handler) => (await events).listen<AppEvent>(CAPTURE_EVENT, (e) => handler(e.payload)),
  };
}

export const api: Api = inTauri() ? createTauriApi() : createMockApi();
