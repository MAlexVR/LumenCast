<script lang="ts">
  import { t, type MessageKey } from "../i18n/index.svelte";
  import { meterPercent, withEffect, withMicrophone, withSpeakers, type AudioState } from "../logic/audio";
  import type { AudioDevice, AudioEffect, AudioPath, AudioSettings, MicProfile, PathStatus } from "../types";
  import Icon from "./Icon.svelte";
  import Segmented from "./Segmented.svelte";
  import Switch from "./Switch.svelte";

  let {
    settings,
    devices,
    state,
    onchange,
    onrefresh,
  }: {
    settings: AudioSettings;
    devices: AudioDevice[];
    state: AudioState;
    onchange: (next: AudioSettings) => void;
    onrefresh: () => void;
  } = $props();

  const sources = $derived(devices.filter((device) => device.kind === "source"));
  const sinks = $derived(devices.filter((device) => device.kind === "sink"));
  const mic = $derived(settings.microphone);
  const out = $derived(settings.speakers);
  const profiles = $derived([
    { value: "headset" as const, label: t("audio_profile_headset") },
    { value: "laptop" as const, label: t("audio_profile_laptop") },
    { value: "desk" as const, label: t("audio_profile_desk") },
  ]);

  function reason(status: PathStatus): string | null {
    return status.state === "unavailable" ? status.reason : null;
  }

  function pick(event: Event): string | null {
    const value = (event.currentTarget as HTMLSelectElement).value;
    return value === "" ? null : value;
  }
</script>

{#snippet strength(effect: AudioEffect, change: (next: AudioEffect) => void)}
  <label class="field">
    <span class="row">
      <span class="label">{t("strength")}</span>
      <span class="num value">{effect.strength}%</span>
    </span>
    <input type="range" min="0" max="100" step="1" value={effect.strength} oninput={(e) => change(withEffect(effect, { strength: Number(e.currentTarget.value) }))} />
  </label>
{/snippet}

{#snippet effect(label: MessageKey, tip: MessageKey, value: AudioEffect, change: (next: AudioEffect) => void)}
  <Switch checked={value.enabled} label={t(label)} hint={t(tip)} onchange={(on) => change(withEffect(value, { enabled: on }))} />
  {#if value.enabled}
    {@render strength(value, change)}
  {/if}
{/snippet}

{#snippet meters(path: AudioPath, status: PathStatus)}
  {#if status.state === "running"}
    <div class="meters" role="status">
      {#each [["audio_level_in", state.levels[path].inputDb], ["audio_level_out", state.levels[path].outputDb]] as [key, db] (key)}
        <div class="meter">
          <span class="label">{t(key as MessageKey)}</span>
          <span class="bar" role="meter" aria-valuemin="0" aria-valuemax="100" aria-valuenow={meterPercent(db as number)} aria-label={t(key as MessageKey)}>
            <span class="fill" style:width={`${meterPercent(db as number)}%`}></span>
          </span>
        </div>
      {/each}
    </div>
  {:else if reason(status)}
    <p class="problem" role="status"><Icon name="alert" size={16} />{t("audio_problem")} <span class="muted">{reason(status)}</span></p>
  {/if}
{/snippet}

<div class="audio">
  <section class="group">
    <Switch checked={mic.enabled} label={t("audio_microphone")} hint={t("audio_microphone_tip")} onchange={(on) => onchange(withMicrophone(settings, { enabled: on }))} />
    {#if mic.enabled}
      <label class="field">
        <span class="label">{t("audio_source")}</span>
        <select value={mic.source ?? ""} onchange={(e) => onchange(withMicrophone(settings, { source: pick(e) }))}>
          <option value="">{t("audio_default_device")}</option>
          {#each sources as device (device.name)}
            <option value={device.name}>{device.description}</option>
          {/each}
        </select>
      </label>
      {@render effect("audio_noise_removal", "audio_noise_removal_tip", mic.noiseRemoval, (next) => onchange(withMicrophone(settings, { noiseRemoval: next })))}
      {@render effect("audio_room_echo", "audio_room_echo_tip", mic.roomEchoRemoval, (next) => onchange(withMicrophone(settings, { roomEchoRemoval: next })))}
      <Switch checked={mic.studioVoice} label={t("audio_studio_voice")} hint={t("audio_studio_voice_tip")} onchange={(on) => onchange(withMicrophone(settings, { studioVoice: on }))} />
      {#if mic.studioVoice}
        <div class="field">
          <span class="label">{t("audio_profile")}</span>
          <Segmented value={mic.profile} label={t("audio_profile")} options={profiles} onchange={(profile: MicProfile) => onchange(withMicrophone(settings, { profile }))} />
        </div>
      {/if}
      {@render meters("microphone", state.microphone)}
    {/if}
  </section>

  <section class="group">
    <Switch checked={out.enabled} label={t("audio_speakers")} hint={t("audio_speakers_tip")} onchange={(on) => onchange(withSpeakers(settings, { enabled: on }))} />
    {#if out.enabled}
      <label class="field">
        <span class="label">{t("audio_sink")}</span>
        <select value={out.sink ?? ""} onchange={(e) => onchange(withSpeakers(settings, { sink: pick(e) }))}>
          <option value="">{t("audio_default_device")}</option>
          {#each sinks as device (device.name)}
            <option value={device.name}>{device.description}</option>
          {/each}
        </select>
      </label>
      {@render effect("audio_speaker_noise", "audio_speaker_noise_tip", out.noiseRemoval, (next) => onchange(withSpeakers(settings, { noiseRemoval: next })))}
      {@render effect("audio_speaker_echo", "audio_speaker_echo_tip", out.echoRemoval, (next) => onchange(withSpeakers(settings, { echoRemoval: next })))}
      {@render meters("speakers", state.speakers)}
    {/if}
  </section>

  <button class="btn btn-quiet" type="button" onclick={onrefresh}><Icon name="restart" />{t("audio_refresh_devices")}</button>
</div>

<style>
  .audio {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: var(--s-5);
    min-width: 0;
  }

  .group {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    min-width: 0;
    gap: var(--s-3);
    padding: var(--s-4);
    background: var(--surface-2);
    border: 1px solid var(--line-soft);
    border-radius: var(--r-2);
  }

  .field {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    min-width: 0;
    gap: var(--s-2);
  }

  select {
    width: 100%;
    min-width: 0;
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

  input[type="range"] {
    width: 100%;
    accent-color: var(--lumen);
  }

  .meters {
    display: grid;
    gap: var(--s-2);
  }

  .meter {
    display: grid;
    grid-template-columns: 64px minmax(0, 1fr);
    min-width: 0;
    align-items: center;
    gap: var(--s-3);
  }

  .bar {
    display: block;
    height: 6px;
    background: var(--surface-1);
    border: 1px solid var(--line-soft);
    border-radius: var(--r-1, 2px);
    overflow: hidden;
  }

  .fill {
    display: block;
    height: 100%;
    background: var(--lumen);
    transition: width 80ms linear;
  }

  .problem {
    display: flex;
    gap: var(--s-2);
    align-items: baseline;
    font-size: var(--fs-xs);
    color: var(--danger);
  }

  .muted {
    color: var(--text-muted);
  }
</style>
