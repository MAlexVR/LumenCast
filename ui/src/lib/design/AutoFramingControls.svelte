<script lang="ts">
  import Segmented from "./Segmented.svelte";
  import { t } from "../i18n/index.svelte";
  import { autoStatusKey, withAutoFraming, zoomFromLevel, zoomLevel } from "../logic/framing";
  import type { AutoFramingStatus, ProcessingSettings } from "../types";

  let {
    settings,
    status,
    running,
    onchange,
  }: { settings: ProcessingSettings; status: AutoFramingStatus | null; running: boolean; onchange: (next: ProcessingSettings) => void } = $props();

  const auto = $derived(settings.autoFraming);
  const statusKey = $derived(running && auto.enabled ? autoStatusKey(status) : null);
</script>

<div class="auto">
  <h2 class="eyebrow">{t("framing_mode")}</h2>

  <Segmented
    value={auto.enabled ? "auto" : "manual"}
    label={t("framing_mode")}
    options={[
      { value: "auto", label: t("mode_auto") },
      { value: "manual", label: t("mode_manual") },
    ]}
    onchange={(next) => onchange(withAutoFraming(settings, { enabled: next === "auto" }))}
  />
  <p class="hint">{auto.enabled ? t("auto_enable_tip") : t("manual_tip")}</p>

  {#if auto.enabled}
    <label class="field">
      <span class="row">
        <span class="label">{t("auto_min_zoom")}</span>
        <span class="num value">{zoomLevel(auto.minZoomPercent)}%</span>
      </span>
      <input
        type="range"
        min="10"
        max="100"
        step="1"
        value={zoomLevel(auto.minZoomPercent)}
        oninput={(e) => onchange(withAutoFraming(settings, { minZoomPercent: zoomFromLevel(Number(e.currentTarget.value)) }))}
      />
    </label>


    <label class="field">
      <span class="row">
        <span class="label">{t("auto_speed")}</span>
        <span class="num value">{auto.speed}%</span>
      </span>
      <input
        type="range"
        min="0"
        max="100"
        step="1"
        value={auto.speed}
        oninput={(e) => onchange(withAutoFraming(settings, { speed: Number(e.currentTarget.value) }))}
      />
      <span class="hint">{t("auto_speed_tip")}</span>
    </label>

    {#if statusKey}
      <p class="status" class:problem={status?.state === "unavailable"} role="status" aria-live="polite">
        {t(statusKey, { zoom: status?.state === "locked" ? zoomLevel(status.zoomPercent) : 0 })}
      </p>
    {/if}
  {/if}
</div>

<style>
  .auto {
    display: grid;
    gap: var(--s-4);
  }

  .field {
    display: grid;
    gap: var(--s-2);
  }

  .row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }

  .label {
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }


  .value {
    font-size: var(--fs-sm);
  }

  .hint {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }



  input[type="range"],

  input[type="range"] {
    width: 100%;
  }

  .status {
    font-size: var(--fs-sm);
    color: var(--ok);
  }

  .status.problem {
    color: var(--danger);
  }
</style>
