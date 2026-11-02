<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import Switch from "./Switch.svelte";
  import { withBackground, withDenoise, withKeyLight } from "../logic/framing";
  import { api } from "../api";
  import Segmented from "./Segmented.svelte";
  import type { BackgroundMode, BackgroundStatus, ProcessingSettings } from "../types";

  let {
    settings,
    background,
    onchange,
  }: { settings: ProcessingSettings; background: BackgroundStatus | null; onchange: (next: ProcessingSettings) => void } = $props();

  let lastVignette = $state(0.3);
  function fileName(path: string): string {
    return path.split(/[\\/]/).pop() || path;
  }

  async function chooseLight() {
    try {
      const path = await api.pickImage();
      if (path) onchange(withKeyLight(settings, { lightPicture: path }));
    } catch {
      // The picker failing leaves the light as it was.
    }
  }

  async function choose() {
    try {
      const path = await api.pickImage();
      if (path) onchange(withBackground(settings, { imagePath: path }));
    } catch {
      // The picker failing is not worth blocking the panel; the old picture stays.
    }
  }

  let lastMode = $state<Exclude<BackgroundMode, "none">>("blur");
  const bg = $derived(settings.background);
  const engines = $derived([
    { value: "nvidia" as const, label: t("bg_engine_nvidia") },
    { value: "builtin" as const, label: t("bg_engine_builtin") },
  ]);
  const modes = $derived([
    { value: "blur" as const, label: t("bg_mode_blur") },
    { value: "remove" as const, label: t("bg_mode_remove") },
    { value: "replace" as const, label: t("bg_mode_replace") },
  ]);
</script>

<div class="effects">
  <section class="effect">
    <Switch
      checked={settings.vignette > 0}
      label={t("vignette")}
      hint={t("vignette_tip")}
      onchange={(on) => {
        if (!on) lastVignette = settings.vignette;
        onchange({ ...settings, vignette: on ? lastVignette : 0 });
      }}
    />
    {#if settings.vignette > 0}
      <label class="field">
        <span class="row">
          <span class="label">{t("strength")}</span>
          <span class="num value">{Math.round(settings.vignette * 100)}%</span>
        </span>
        <input
          type="range"
          min="1"
          max="100"
          step="1"
          value={Math.round(settings.vignette * 100)}
          oninput={(e) => onchange({ ...settings, vignette: Number(e.currentTarget.value) / 100 })}
        />
      </label>
      <Switch
        checked={settings.vignetteFollowsFace}
        label={t("vignette_follow")}
        hint={t("vignette_follow_tip")}
        onchange={(on) => onchange({ ...settings, vignetteFollowsFace: on })}
      />
    {/if}
  </section>

  <section class="effect">
    <Switch
      checked={settings.keyLight.enabled}
      label={t("effect_key_light")}
      hint={t("effect_key_light_tip")}
      onchange={(on) => onchange(withKeyLight(settings, { enabled: on }))}
    />
    {#if settings.keyLight.enabled}
      <label class="field">
        <span class="row">
          <span class="label">{t("strength")}</span>
          <span class="num value">{settings.keyLight.intensity}%</span>
        </span>
        <input type="range" min="1" max="100" step="1" value={settings.keyLight.intensity} oninput={(e) => onchange(withKeyLight(settings, { intensity: Number(e.currentTarget.value) }))} />
      </label>
      <label class="field">
        <span class="row">
          <span class="label">{t("warmth")}</span>
          <span class="num value">{settings.keyLight.warmth}%</span>
        </span>
        <input type="range" min="0" max="100" step="1" value={settings.keyLight.warmth} oninput={(e) => onchange(withKeyLight(settings, { warmth: Number(e.currentTarget.value) }))} />
        <span class="scale"><span>{t("warmth_cool")}</span><span>{t("warmth_warm")}</span></span>
      </label>
      <div class="field">
        <span class="label" id="light-picture-label">{t("light_picture")}</span>
        <div class="picker">
          <span class="picked" class:empty={!settings.keyLight.lightPicture} title={settings.keyLight.lightPicture ?? ""} aria-labelledby="light-picture-label">
            {settings.keyLight.lightPicture ? fileName(settings.keyLight.lightPicture) : t("light_picture_none")}
          </span>
          <button class="btn btn-quiet" type="button" onclick={chooseLight}>{t("bg_choose")}</button>
          {#if settings.keyLight.lightPicture}
            <button class="btn btn-quiet" type="button" onclick={() => onchange(withKeyLight(settings, { lightPicture: null }))}>{t("light_picture_clear")}</button>
          {/if}
        </div>
        <span class="hint">{t("light_picture_tip")}</span>
      </div>
    {/if}
  </section>

  <section class="effect">
    <Switch
      checked={bg.mode !== "none"}
      label={t("effect_background")}
      hint={t("effect_background_tip")}
      onchange={(on) => onchange(withBackground(settings, { mode: on ? lastMode : "none" }))}
    />
    {#if bg.mode !== "none"}
      <Segmented
        value={bg.mode}
        label={t("effect_background")}
        options={modes}
        onchange={(mode) => {
          lastMode = mode;
          onchange(withBackground(settings, { mode }));
        }}
      />
      <Segmented
        value={bg.engine}
        label={t("bg_engine")}
        options={engines}
        onchange={(engine) => onchange(withBackground(settings, { engine }))}
      />
      {#if background?.state === "working"}
        <p class="hint" role="status">
          {background.engine === "nvidia" ? t("bg_using_nvidia") : t("bg_using_builtin")}
          {#if background.fallback}<span class="muted">· {t("bg_fallback")} {background.fallback}</span>{/if}
        </p>
      {/if}
      {#if bg.mode === "blur"}
        <label class="field">
          <span class="row">
            <span class="label">{t("bg_blur")}</span>
            <span class="num value">{bg.blurStrength}%</span>
          </span>
          <input type="range" min="0" max="100" step="1" value={bg.blurStrength} oninput={(e) => onchange(withBackground(settings, { blurStrength: Number(e.currentTarget.value) }))} />
        </label>
      {:else if bg.mode === "remove"}
        <span class="hint">{t("bg_remove_tip")}</span>
      {:else}
        <div class="field">
          <span class="label" id="bg-image-label">{t("bg_image")}</span>
          <div class="picker">
            <span class="picked" class:empty={!bg.imagePath} title={bg.imagePath ?? ""} aria-labelledby="bg-image-label">
              {bg.imagePath ? fileName(bg.imagePath) : t("bg_no_image")}
            </span>
            <button class="btn btn-quiet" type="button" onclick={choose}>{t("bg_choose")}</button>
          </div>
        </div>
      {/if}
      {#if background?.state === "unavailable" && background.problem === "noPicture"}
        <p class="hint" role="status">{t("bg_pick_first")}</p>
      {:else if background?.state === "unavailable"}
        <p class="problem" role="status">{t(background.problem === "pictureUnreadable" ? "bg_picture_unreadable" : "bg_unavailable")} <span class="muted">{background.reason}</span></p>
      {/if}
    {/if}
  </section>

  <section class="effect">
    <Switch
      checked={settings.denoise.enabled}
      label={t("effect_noise")}
      hint={t("effect_noise_tip")}
      onchange={(on) => onchange(withDenoise(settings, { enabled: on }))}
    />
    {#if settings.denoise.enabled}
      <label class="field">
        <span class="row">
          <span class="label">{t("strength")}</span>
          <span class="num value">{settings.denoise.strength}%</span>
        </span>
        <input type="range" min="1" max="100" step="1" value={settings.denoise.strength} oninput={(e) => onchange(withDenoise(settings, { strength: Number(e.currentTarget.value) }))} />
      </label>
    {/if}
  </section>

</div>

<style>
  .effects {
    display: grid;
    gap: var(--s-5);
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

  .picker {
    display: flex;
    gap: var(--s-2);
    align-items: center;
  }

  .picked {
    flex: 1;
    min-width: 0;
    height: 36px;
    display: flex;
    align-items: center;
    padding: 0 var(--s-3);
    background: var(--surface-1);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .picked.empty {
    color: var(--text-muted);
  }

  .problem {
    font-size: var(--fs-xs);
    color: var(--danger);
  }

  .muted {
    color: var(--text-muted);
  }

  .scale {
    display: flex;
    justify-content: space-between;
    font-size: var(--fs-xs);
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
    width: 100%;
    accent-color: var(--lumen);
  }

  .effect {
    display: grid;
    gap: var(--s-3);
    padding: var(--s-4);
    background: var(--surface-2);
    border: 1px solid var(--line-soft);
    border-radius: var(--r-2);
  }

  section {
    display: grid;
    gap: var(--s-2);
  }




</style>
