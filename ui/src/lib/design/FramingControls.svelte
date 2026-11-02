<script lang="ts">
  import Icon from "./Icon.svelte";
  import { t } from "../i18n/index.svelte";
  import { canPan, centerPan, nudgePan, withZoom, zoomFromLevel, zoomLevel, type PanDirection } from "../logic/framing";
  import type { ProcessingSettings } from "../types";

  let { settings, onchange }: { settings: ProcessingSettings; onchange: (next: ProcessingSettings) => void } = $props();

  const pannable = $derived(canPan(settings));
  const pad: { direction: PanDirection; icon: "arrowUp" | "arrowDown" | "arrowLeft" | "arrowRight"; label: "pan_up" | "pan_down" | "pan_left" | "pan_right" }[] = [
    { direction: "up", icon: "arrowUp", label: "pan_up" },
    { direction: "left", icon: "arrowLeft", label: "pan_left" },
    { direction: "right", icon: "arrowRight", label: "pan_right" },
    { direction: "down", icon: "arrowDown", label: "pan_down" },
  ];
</script>

<div class="framing">
  <h2 class="eyebrow">{t("section_manual")}</h2>

  <label class="field">
    <span class="row">
      <span class="label">{t("zoom")}</span>
      <span class="num value">{zoomLevel(settings.zoomPercent)}%</span>
    </span>
    <input
      type="range"
      min="0"
      max="100"
      step="1"
      value={zoomLevel(settings.zoomPercent)}
      oninput={(e) => onchange(withZoom(settings, zoomFromLevel(Number(e.currentTarget.value))))}
    />
  </label>

  <div class="field">
    <span class="label" id="pan-label">{t("pan")}</span>
    <div class="pad" role="group" aria-labelledby="pan-label">
      {#each pad as key (key.direction)}
        <button
          class="icon-btn key {key.direction}"
          type="button"
          aria-label={t(key.label)}
          disabled={!pannable}
          onclick={() => onchange(nudgePan(settings, key.direction))}
        >
          <Icon name={key.icon} />
        </button>
      {/each}
      <button class="icon-btn key center" type="button" aria-label={t("pan_center")} disabled={!pannable} onclick={() => onchange(centerPan(settings))}>
        <Icon name="center" />
      </button>
    </div>
    {#if !pannable}<p class="hint">{t("pan_needs_zoom")}</p>{/if}
  </div>
</div>

<style>
  .framing {
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

  input[type="range"] {
    accent-color: var(--lumen);
  }

  input[type="range"] {
    width: 100%;
  }

  .pad {
    position: relative;
    justify-self: center;
    width: 136px;
    height: 136px;
    border-radius: 50%;
    background: radial-gradient(circle at center, var(--surface-3), var(--surface-2));
    border: 1px solid var(--line);
  }

  .key {
    position: absolute;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    color: var(--text-muted);
    transition:
      background-color var(--dur-2) var(--ease-out),
      color var(--dur-2) var(--ease-out);
  }

  .key:hover:not(:disabled) {
    background: var(--surface-1);
    color: var(--lumen);
  }

  .key:disabled {
    opacity: 0.35;
  }

  .up {
    top: 4px;
    left: calc(50% - 20px);
  }

  .down {
    bottom: 4px;
    left: calc(50% - 20px);
  }

  .left {
    left: 4px;
    top: calc(50% - 20px);
  }

  .right {
    right: 4px;
    top: calc(50% - 20px);
  }

  .center {
    left: calc(50% - 20px);
    top: calc(50% - 20px);
    border: 1px solid var(--line);
    color: var(--lumen);
  }
</style>
