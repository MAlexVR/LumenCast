<script lang="ts">
  import { LOCALES, i18n, t, type Locale } from "../i18n/index.svelte";
  import Switch from "./Switch.svelte";
  import type { ProcessingSettings } from "../types";

  let {
    settings,
    onchange,
    autostart,
    onautostart,
    closeToTray,
    onclosetotray,
    checkUpdates,
    oncheckupdates,
    onlanguage,
  }: {
    settings: ProcessingSettings;
    onchange: (next: ProcessingSettings) => void;
    autostart: boolean;
    onautostart: (enabled: boolean) => void;
    closeToTray: boolean;
    onclosetotray: (enabled: boolean) => void;
    checkUpdates: boolean;
    oncheckupdates: (enabled: boolean) => void;
    onlanguage: (code: Locale) => void;
  } = $props();
</script>

<div class="general">
  <Switch checked={autostart} label={t("autostart")} hint={t("autostart_tip")} onchange={onautostart} />
  <Switch checked={closeToTray} label={t("close_to_tray")} hint={t("close_to_tray_tip")} onchange={onclosetotray} />
  <Switch checked={checkUpdates} label={t("check_updates_on_start")} hint={t("check_updates_on_start_tip")} onchange={oncheckupdates} />

  <label class="field">
    <span class="label">{t("language")}</span>
    <select value={i18n.locale} onchange={(e) => onlanguage(e.currentTarget.value as Locale)}>
      {#each LOCALES as locale (locale)}
        <option value={locale}>{locale === "es" ? "Español" : "English"}</option>
      {/each}
    </select>
  </label>

  <p class="hint about">{t("about_text")}</p>
</div>

<style>
  .general {
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

  .about {
    margin-top: var(--s-4);
  }
</style>
