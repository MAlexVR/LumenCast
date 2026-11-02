<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import Switch from "./Switch.svelte";
  import type { ProcessingSettings, VirtualCameraStatus } from "../types";

  let {
    settings,
    onchange,
    virtualCamera,
  }: {
    settings: ProcessingSettings;
    onchange: (next: ProcessingSettings) => void;
    virtualCamera: VirtualCameraStatus | null;
  } = $props();
</script>

<div class="output-panel">
  <Switch checked={settings.mirrorPreview} label={t("mirror_preview")} hint={t("mirror_tip")} onchange={(next) => onchange({ ...settings, mirrorPreview: next })} />

  <section class="output">
    <Switch
      checked={settings.virtualCamera.enabled}
      label={t("virtual_camera")}
      hint={t("virtual_camera_tip")}
      onchange={(on) => onchange({ ...settings, virtualCamera: { ...settings.virtualCamera, enabled: on } })}
    />
    {#if settings.virtualCamera.enabled}
      <label class="field">
        <span class="label">{t("output_scale")}</span>
        <select
          value={settings.virtualCamera.upscaleHeight}
          onchange={(e) => onchange({ ...settings, virtualCamera: { ...settings.virtualCamera, upscaleHeight: Number(e.currentTarget.value) } })}
        >
          <option value={0}>{t("output_scale_none")}</option>
          <option value={1080}>1080p</option>
          <option value={1440}>1440p</option>
          <option value={2160}>2160p</option>
        </select>
        <span class="hint">{t("output_scale_tip")}</span>
      </label>
    {/if}
    {#if settings.virtualCamera.enabled && virtualCamera?.state === "active"}
      <p class="hint" role="status">{t("virtual_camera_active", { device: virtualCamera.device, size: `${virtualCamera.width}×${virtualCamera.height}` })}</p>
    {:else if settings.virtualCamera.enabled && virtualCamera?.state === "unavailable"}
      <p class="problem" role="status">{t("virtual_camera_unavailable")} <span class="muted">{virtualCamera.reason}</span></p>
    {:else if settings.virtualCamera.enabled}
      <p class="hint" role="status">{t("virtual_camera_waiting")}</p>
    {/if}
  </section>
</div>

<style>
  .output-panel {
    display: grid;
    gap: var(--s-5);
  }

  .field {
    display: grid;
    gap: var(--s-2);
  }

  .label {
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }

  .hint {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }

  select {
    width: 100%;
    min-width: 0;
    height: 40px;
    padding: 0 var(--s-3);
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
  }

  .output {
    display: grid;
    gap: var(--s-2);
  }

  .problem {
    font-size: var(--fs-xs);
    color: var(--danger);
  }

  .muted {
    color: var(--text-muted);
  }
</style>
