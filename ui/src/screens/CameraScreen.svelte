<script lang="ts">
  import { onMount } from "svelte";
  import AudioPanel from "../lib/design/AudioPanel.svelte";
  import CameraOutput from "../lib/design/CameraOutput.svelte";
  import AutoFramingControls from "../lib/design/AutoFramingControls.svelte";
  import EffectsPanel from "../lib/design/EffectsPanel.svelte";
  import FramingControls from "../lib/design/FramingControls.svelte";
  import GeneralPanel from "../lib/design/GeneralPanel.svelte";
  import UpdatePanel from "../lib/design/UpdatePanel.svelte";
  import ImportNotice from "../lib/design/ImportNotice.svelte";
  import Icon from "../lib/design/Icon.svelte";
  import Rail from "../lib/components/Rail.svelte";
  import { version } from "../../package.json";
  import { api } from "../lib/api";
  import { setLocale, t, type Locale, type MessageKey } from "../lib/i18n/index.svelte";
  import { DEFAULT_AUDIO, initialAudioState, reduceAudio, audioInRange, type AudioState } from "../lib/logic/audio";
  import { initialState, reduce, type Action, type CaptureState } from "../lib/logic/captureState";
  import { DEFAULT_PROCESSING } from "../lib/logic/framing";
  import { createLatestSender } from "../lib/logic/latest";
  import { errorMessageKey, toErrorPayload } from "../lib/logic/errors";
  import { fpsLabel, initialSelection, optionsFor, pickCamera, preferredSelection, previewRatio, reconcile, sizeLabel, type Selection } from "../lib/logic/modes";
  import { endMessageKey, fpsText, negotiationRows, previewShown, previewStalled } from "../lib/logic/view";
  import type { AudioDevice, AudioSettings, CameraInfo, CameraProbe, ErrorPayload, ImportReport, PersistedSettings, ProcessingSettings, UpdateProgress, UpdateStatus } from "../lib/types";

  let cameras = $state<CameraInfo[]>([]);
  let cameraIndex = $state<number | null>(null);
  let probe = $state<CameraProbe | null>(null);
  let selection = $state<Selection | null>(null);
  let capture = $state<CaptureState>(initialState);
  let loadError = $state<ErrorPayload | null>(null);
  let searched = $state(false);
  type TabId = "camera" | "effects" | "framing" | "audio" | "general";
  let tab = $state<TabId>("camera");
  const tabs = $derived([
    { id: "camera" as const, icon: "camera" as const, label: t("tab_camera") },
    { id: "effects" as const, icon: "effects" as const, label: t("tab_effects") },
    { id: "framing" as const, icon: "framing" as const, label: t("tab_framing") },
    { id: "audio" as const, icon: "audio" as const, label: t("tab_audio") },
    { id: "general" as const, icon: "settings" as const, label: t("tab_general") },
  ]);
  let processing = $state<ProcessingSettings>({ ...DEFAULT_PROCESSING });
  let audio = $state<AudioSettings>(structuredClone(DEFAULT_AUDIO));
  let audioDevices = $state<AudioDevice[]>([]);
  let audioState = $state<AudioState>(initialAudioState);
  let saved = $state<PersistedSettings | null>(null);
  let importReport = $state<ImportReport | null>(null);
  let settingsProblem = $state<string | null>(null);
  /** The saved camera and mode apply to the first selection only. */
  let useSaved = true;

  const modes = $derived(probe?.capabilities.modes ?? []);
  const options = $derived(optionsFor(modes, { format: selection?.format, size: selection?.size }));
  const running = $derived(capture.phase === "starting" || capture.phase === "running");
  // A capture that started and then failed carries its reason in the outcome.
  const failure = $derived(capture.outcome?.end.kind === "failed" ? capture.outcome.end.error : null);
  const shownError = $derived(capture.error ?? failure ?? loadError);
  const phaseKey = $derived(`state_${capture.phase}` as MessageKey);
  const rows = $derived(capture.report ? negotiationRows(capture.report) : []);
  const adjusted = $derived((capture.report?.differences.length ?? 0) > 0);
  const ratio = $derived(previewRatio(capture.report?.negotiated.size ?? selection?.size ?? null));
  const stalled = $derived(running && !!capture.stats && previewStalled(capture.stats, capture.previewDropped));
  const previewSrc = $derived(capture.previewBase64 ? `data:image/jpeg;base64,${capture.previewBase64}` : null);

  const sendProcessing = createLatestSender(
    (next: ProcessingSettings) => api.setProcessing(next),
    (error) => (loadError = toErrorPayload(error)),
  );

  function changeProcessing(next: ProcessingSettings) {
    processing = next;
    sendProcessing(next);
  }

  const sendAudio = createLatestSender(
    (next: AudioSettings) => api.setAudio(next),
    (error) => (loadError = toErrorPayload(error)),
  );

  function changeAudio(next: AudioSettings) {
    if (!audioInRange(next)) return;
    audio = next;
    sendAudio(next);
  }

  async function refreshAudioDevices() {
    try {
      audioDevices = await api.listAudioDevices();
    } catch (error) {
      audioDevices = [];
      loadError = toErrorPayload(error);
    }
  }

  async function loadAudio() {
    try {
      audio = await api.getAudio();
      audioState = { ...audioState, ...(await api.getAudioStatus()) };
    } catch (error) {
      loadError = toErrorPayload(error);
    }
    // A machine without PipeWire still has every other panel.
    await refreshAudioDevices();
  }

  function resetDefaults() {
    changeProcessing({ ...DEFAULT_PROCESSING });
  }

  function dispatch(action: Action) {
    capture = reduce(capture, action);
  }

  async function loadProbe(index: number) {
    probe = null;
    selection = null;
    try {
      probe = await api.probeCamera(index);
      selection = useSaved
        ? preferredSelection(probe.capabilities.modes, saved?.preferredMode ?? null, saved?.modeHint ?? null)
        : initialSelection(probe.capabilities.modes);
      useSaved = false;
      loadError = null;
    } catch (error) {
      loadError = toErrorPayload(error);
    }
  }

  async function refresh() {
    try {
      cameras = await api.listCameras();
      loadError = null;
    } catch (error) {
      cameras = [];
      loadError = toErrorPayload(error);
    }
    searched = true;
    cameraIndex = pickCamera(cameras, saved?.preferredCamera ?? null, cameraIndex)?.index ?? null;
    if (cameraIndex !== null) await loadProbe(cameraIndex);
  }

  async function onCamera(event: Event) {
    cameraIndex = Number((event.currentTarget as HTMLSelectElement).value);
    useSaved = false;
    await loadProbe(cameraIndex);
  }

  function onFormat(event: Event) {
    if (selection) selection = reconcile(modes, selection, { format: (event.currentTarget as HTMLSelectElement).value });
  }

  function onSize(event: Event) {
    const size = options.sizes[Number((event.currentTarget as HTMLSelectElement).value)];
    if (selection && size) selection = reconcile(modes, selection, { size });
  }

  function onInterval(event: Event) {
    const interval = options.intervals[Number((event.currentTarget as HTMLSelectElement).value)];
    if (selection && interval) selection = reconcile(modes, selection, { interval });
  }

  let autostart = $state(false);
  let closeToTray = $state(true);
  let checkUpdates = $state(true);
  let updateStatus = $state<UpdateStatus | null>(null);
  let updateChecking = $state(false);
  let updateInstalling = $state(false);
  let updateProgress = $state<UpdateProgress | null>(null);
  let updateError = $state<ErrorPayload | null>(null);
  /** The "new version" notice above the preview; closed for good once looked at. */
  let updateNotice = $state(false);

  /** Asks GitHub; the check at start is silent when it fails, the one asked for is not. */
  async function checkForUpdate(manual: boolean) {
    updateChecking = true;
    if (manual) updateError = null;
    try {
      updateStatus = await api.checkUpdate();
      updateNotice = !manual && !!updateStatus.available;
    } catch (error) {
      if (manual) updateError = toErrorPayload(error);
    } finally {
      updateChecking = false;
    }
  }

  async function installUpdate() {
    updateError = null;
    updateInstalling = true;
    updateProgress = null;
    try {
      await api.installUpdate();
    } catch (error) {
      updateError = toErrorPayload(error);
      updateInstalling = false;
    }
  }

  function cancelUpdate() {
    api.cancelUpdate().catch(() => {});
  }

  function showUpdates() {
    tab = "general";
    updateNotice = false;
  }

  async function changeCloseToTray(enabled: boolean) {
    const before = closeToTray;
    closeToTray = enabled;
    try {
      await api.setCloseToTray(enabled);
    } catch (error) {
      closeToTray = before;
      loadError = toErrorPayload(error);
    }
  }

  async function changeCheckUpdates(enabled: boolean) {
    const before = checkUpdates;
    checkUpdates = enabled;
    try {
      await api.setCheckUpdates(enabled);
    } catch (error) {
      checkUpdates = before;
      loadError = toErrorPayload(error);
    }
  }

  /** The tray menu changed the virtual camera or the autostart: read them again. */
  async function reloadFromTray() {
    try {
      processing = await api.getProcessing();
      autostart = await api.getAutostart();
    } catch {
      // The window keeps what it had; the next change of its own fixes it.
    }
  }

  async function changeAutostart(enabled: boolean) {
    const before = autostart;
    autostart = enabled;
    try {
      await api.setAutostart(enabled);
    } catch (error) {
      autostart = before;
      loadError = toErrorPayload(error);
    }
  }

  async function start() {
    if (cameraIndex === null || !selection) return;
    dispatch({ type: "start" });
    try {
      await api.startCapture(cameraIndex, { format: selection.format, size: selection.size, interval: selection.interval });
    } catch (error) {
      dispatch({ type: "rejected", error: toErrorPayload(error) });
    }
  }

  async function stop() {
    try {
      await api.stopCapture();
    } catch (error) {
      dispatch({ type: "rejected", error: toErrorPayload(error) });
    }
  }

  const sizeIndex = $derived(selection ? options.sizes.findIndex((s) => s.width === selection!.size.width && s.height === selection!.size.height) : -1);
  const intervalIndex = $derived(
    selection ? options.intervals.findIndex((i) => i.numerator * selection!.interval.denominator === selection!.interval.numerator * i.denominator) : -1,
  );

  async function startUp() {
    try {
      const preferences = await api.getPreferences();
      saved = preferences.settings;
      processing = preferences.settings.processing;
      importReport = preferences.importReport;
      closeToTray = preferences.settings.closeToTray;
      checkUpdates = preferences.settings.checkUpdates;
      settingsProblem = preferences.problem;
      if (preferences.settings.language) setLocale(preferences.settings.language);
    } catch (error) {
      loadError = toErrorPayload(error);
    }
    try {
      autostart = await api.getAutostart();
    } catch {
      autostart = false;
    }
    await refresh();
  }

  function chooseLanguage(code: Locale) {
    setLocale(code);
    api.setLanguage(code).catch((error) => (loadError = toErrorPayload(error)));
  }

  onMount(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void api
      .onCaptureEvent((event) => {
        // Audio is independent of the capture: its events never wait for a session.
        audioState = reduceAudio(audioState, event);
        dispatch({ type: "event", event });
      })
      .then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    const unsubscribe: (() => void)[] = [];
    const listen = (subscription: Promise<() => void>) =>
      void subscription.then((fn) => {
        if (cancelled) fn();
        else unsubscribe.push(fn);
      });
    listen(api.onUpdateProgress((progress) => (updateProgress = progress)));
    listen(api.onTrayChanged(() => void reloadFromTray()));
    listen(
      api.onTrayUpdates(() => {
        tab = "general";
        void checkForUpdate(true);
      }),
    );
    void startUp().then(() => {
      if (checkUpdates) void checkForUpdate(false);
    });
    void loadAudio();
    return () => {
      cancelled = true;
      unlisten?.();
      unsubscribe.forEach((fn) => fn());
    };
  });
</script>

<div class="screen">
  <Rail items={tabs} selected={tab} {version} onselect={(id) => (tab = id)} />

  <main class="body">
    <section class="panel" aria-label={t("tabs_label")}>
      <div class="tabpanel" role="tabpanel" id={`panel-${tab}`} aria-labelledby="panel-title" tabindex="0">
        <h1 id="panel-title" class="panel-title">{tabs.find((entry) => entry.id === tab)?.label}</h1>
        {#if tab === "camera"}
          {#if searched && cameras.length === 0}
            <div class="empty">
              <p>{t("camera_none")}</p>
              <p class="muted">{t("camera_none_hint")}</p>
            </div>
          {:else}
            <label class="field">
              <span class="label">{t("camera")}</span>
              <select value={cameraIndex ?? ""} onchange={onCamera} disabled={running || cameras.length === 0}>
                {#each cameras as camera (camera.index)}
                  <option value={camera.index}>{camera.name ?? camera.path}</option>
                {/each}
              </select>
            </label>
          {/if}

          <button class="btn btn-quiet" type="button" onclick={refresh} disabled={running}>
            <Icon name="restart" />{t("refresh")}
          </button>

          {#if selection}
            <h2 class="eyebrow section">{t("mode")}</h2>
            <label class="field">
              <span class="label">{t("format")}</span>
              <select value={selection.format} onchange={onFormat} disabled={running}>
                {#each options.formats as format (format)}
                  <option value={format}>{format}</option>
                {/each}
              </select>
            </label>
            <label class="field">
              <span class="label">{t("size")}</span>
              <select value={sizeIndex} onchange={onSize} disabled={running}>
                {#each options.sizes as size, index (index)}
                  <option value={index}>{sizeLabel(size)}</option>
                {/each}
              </select>
            </label>
            <label class="field">
              <span class="label">{t("frame_rate")}</span>
              <select value={intervalIndex} onchange={onInterval} disabled={running}>
                {#each options.intervals as interval, index (index)}
                  <option value={index}>{fpsLabel(interval)}</option>
                {/each}
              </select>
            </label>
          {/if}

          <h2 class="eyebrow section">{t("output")}</h2>
          <CameraOutput settings={processing} virtualCamera={capture.virtualCamera} onchange={changeProcessing} />
        {:else if tab === "effects"}
          <EffectsPanel settings={processing} background={capture.background} onchange={changeProcessing} />
        {:else if tab === "framing"}
          <AutoFramingControls settings={processing} status={capture.autoFraming} {running} onchange={changeProcessing} />
          {#if !processing.autoFraming.enabled}
            <FramingControls settings={processing} onchange={changeProcessing} />
          {/if}
        {:else if tab === "audio"}
          <AudioPanel settings={audio} devices={audioDevices} state={audioState} onchange={changeAudio} onrefresh={refreshAudioDevices} />
        {:else}
          <GeneralPanel settings={processing} {autostart} onautostart={changeAutostart} {closeToTray} onclosetotray={changeCloseToTray} {checkUpdates} oncheckupdates={changeCheckUpdates} onchange={changeProcessing} onlanguage={chooseLanguage} onreset={resetDefaults} />
          <UpdatePanel
            status={updateStatus}
            {version}
            checking={updateChecking}
            progress={updateProgress}
            installing={updateInstalling}
            error={updateError}
            oncheck={() => void checkForUpdate(true)}
            oninstall={() => void installUpdate()}
            oncancel={cancelUpdate}
          />
        {/if}
      </div>
    </section>

    <div class="content">
    {#if importReport}
      <div class="banner"><ImportNotice report={importReport} ondismiss={() => (importReport = null)} /></div>
    {/if}
    {#if updateNotice && updateStatus?.available}
      <p class="banner notice" role="status">
        <Icon name="arrowDown" />{t("update_available_banner", { version: updateStatus.available.version })}
        <button class="btn btn-quiet" type="button" onclick={showUpdates}>{t("update_see")}</button>
        <button class="btn btn-quiet" type="button" onclick={() => (updateNotice = false)}>{t("import_dismiss")}</button>
      </p>
    {/if}
    {#if settingsProblem}
      <p class="banner error" role="alert"><Icon name="alert" />{t("settings_problem")} <span class="muted">{settingsProblem}</span></p>
    {/if}
    <section class="stage" aria-label={t("preview_label")}>
      <div class="frame" style:--ratio={ratio}>
        {#if previewSrc && !stalled}
          <img src={previewSrc} alt={t("preview_label")} />
        {:else if stalled}
          <p class="muted">{t("preview_stalled")}</p>
        {:else}
          <p class="muted">{t("preview_idle")}</p>
        {/if}
      </div>
      <div class="stagebar">
        <p class="phase" role="status" aria-live="polite">
          <span class="dot" data-phase={capture.phase}></span>{t(phaseKey)}
          {#if capture.outcome}<span class="muted"> · {t(endMessageKey(capture.outcome.end))}</span>{/if}
        </p>
        {#if running}
          <button class="btn btn-stop" type="button" onclick={stop}><Icon name="stop" />{t("stop")}</button>
        {:else}
          <button class="btn btn-primary" class:ready={!!selection} type="button" onclick={start} disabled={!selection}>
            <Icon name="play" />{t("start")}
          </button>
        {/if}
      </div>
      {#if shownError}
        <p class="error" role="alert"><Icon name="alert" />{t(errorMessageKey(shownError.code))}</p>
      {/if}
    </section>

    <section class="status" aria-label={t("negotiated")}>
      {#if capture.report}
        <div class="negotiation">
        <table>
          <thead>
            <tr><th scope="col"><span class="visually-hidden">{t("mode")}</span></th><th scope="col" class="eyebrow">{t("requested")}</th><th scope="col" class="eyebrow">{t("negotiated")}</th></tr>
          </thead>
          <tbody>
            {#each rows as row (row.field)}
              <tr class:differs={row.differs}>
                <th scope="row">{t(row.labelKey)}</th>
                <td class="num">{row.requested}</td>
                <td class="num">{row.negotiated}{#if row.differs}<span class="visually-hidden"> ≠</span>{/if}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        <p class="note" class:warn={adjusted}>
          <Icon name={adjusted ? "alert" : "check"} size={16} />{adjusted ? t("adjusted") : t("exact_match")}
        </p>
        </div>
      {/if}

      {#if capture.stats}
        <dl class="counters">
          <div><dt class="eyebrow">{t("measured_fps")}</dt><dd class="num big">{fpsText(capture.stats)}</dd></div>
          <div><dt class="eyebrow">{t("frames_captured")}</dt><dd class="num">{capture.stats.framesCaptured}</dd></div>
          <div><dt class="eyebrow">{t("frames_previewed")}</dt><dd class="num">{previewShown(capture.stats, capture.previewDropped)}</dd></div>
          <div><dt class="eyebrow">{t("decode_errors")}</dt><dd class="num">{capture.stats.decodeErrors}</dd></div>
          {#if capture.previewDropped > 0}
            <div><dt class="eyebrow">{t("preview_dropped")}</dt><dd class="num">{capture.previewDropped}</dd></div>
          {/if}
          <div><dt class="eyebrow">{t("acquisition_errors")}</dt><dd class="num">{capture.stats.acquisitionErrors}</dd></div>
          <div><dt class="eyebrow">{t("out_of_order")}</dt><dd class="num">{capture.stats.outOfOrderTimestamps}</dd></div>
        </dl>
      {/if}

      {#if probe}
        <details class="skipped">
          <summary><Icon name="chevronDown" size={16} />{t("skipped_modes")} ({probe.skipped.length})</summary>
          {#if probe.skipped.length === 0}
            <p class="muted">{t("skipped_none")}</p>
          {:else}
            <ul>
              {#each probe.skipped as skipped, index (index)}
                <li><span class="num">{skipped.format}{skipped.size ? ` ${skipped.size[0]}×${skipped.size[1]}` : ""}</span> {skipped.reason}</li>
              {/each}
            </ul>
          {/if}
        </details>
      {/if}
    </section>
    </div>
  </main>
</div>

<style>
  .screen {
    display: grid;
    grid-template-columns: var(--rail-w) minmax(0, 1fr);
    height: 100%;
    background: var(--bg);
  }



  :global(#app) {
    height: 100%;
  }

  .body {
    display: grid;
    grid-template-columns: minmax(280px, 340px) minmax(0, 1fr);
    min-height: 0;
  }

  .content {
    min-width: 0;
    min-height: 0;
    overflow: auto;
  }

  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    border-right: 1px solid var(--line);
    background: var(--surface-1);
  }

  .panel-title {
    font-family: var(--font-display);
    font-size: var(--fs-lg);
    font-weight: var(--fw-display);
    letter-spacing: -0.02em;
  }

  .tabpanel {
    display: flex;
    flex-direction: column;
    gap: var(--s-4);
    padding: var(--s-5) var(--s-5) var(--s-6);
    overflow: auto;
    min-height: 0;
  }

  .stagebar .btn {
    min-width: 140px;
    justify-content: center;
  }

  .btn-stop {
    color: var(--danger);
    border: 1px solid var(--danger);
    background: color-mix(in srgb, var(--danger) 10%, transparent);
  }

  .btn-stop:hover {
    background: color-mix(in srgb, var(--danger) 20%, transparent);
  }

  .stagebar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-4);
  }

  .section {
    margin-top: var(--s-4);
  }

  .banner {
    margin: var(--s-4) var(--gutter) 0;
  }

  .banner.notice {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s-3);
    align-items: center;
    padding: var(--s-3) var(--s-4);
    border: 1px solid var(--line);
    border-left: 2px solid var(--lumen);
    border-radius: var(--r-2);
    background: var(--surface-1);
    font-size: var(--fs-sm);
  }

  .field {
    display: grid;
    gap: var(--s-2);
  }

  .label {
    font-size: var(--fs-sm);
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

  select:disabled {
    color: var(--text-faint);
  }

  .ready {
    box-shadow: 0 0 24px var(--lumen-glow);
  }

  .empty {
    padding: var(--s-4);
    border: 1px dashed var(--line);
    border-radius: var(--r-2);
  }

  .muted {
    color: var(--text-muted);
  }

  .stage {
    display: grid;
    gap: var(--s-3);
    align-content: start;
    padding: var(--s-6) var(--gutter) var(--s-3);
    min-width: 0;
  }

  /* The box takes the real aspect ratio of the chosen resolution and is as wide
     as the column allows without exceeding the stage height, so the image is
     never cropped or stretched. */
  .frame {
    --stage-h: 52vh;
    position: relative;
    display: grid;
    place-items: center;
    justify-self: center;
    aspect-ratio: var(--ratio, 1.7778);
    width: min(100%, calc(var(--stage-h) * var(--ratio, 1.7778)));
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: var(--r-3);
    overflow: hidden;
  }

  .frame img {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
  }

  .phase {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font-size: var(--fs-sm);
  }

  .dot {
    width: 8px;
    height: 8px;
    border-radius: var(--r-pill);
    background: var(--text-faint);
  }

  .dot[data-phase="running"] {
    background: var(--ok);
  }

  .dot[data-phase="starting"] {
    background: var(--lumen);
  }

  .error {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    color: var(--danger);
  }

  .status {
    display: grid;
    gap: var(--s-5);
    padding: var(--s-3) var(--gutter) var(--s-8);
    min-width: 0;
  }

  .negotiation {
    display: grid;
    gap: var(--s-3);
    min-width: 0;
  }

  table {
    border-collapse: collapse;
    width: 100%;
    max-width: 520px;
  }

  th,
  td {
    text-align: left;
    padding: var(--s-2) var(--s-3) var(--s-2) 0;
    border-bottom: 1px solid var(--line-soft);
    font-weight: 400;
  }

  tbody th {
    color: var(--text-muted);
  }

  tr.differs td {
    color: var(--lumen);
  }

  .note {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    font-size: var(--fs-sm);
    color: var(--ok);
  }

  .note.warn {
    color: var(--lumen);
  }

  .counters {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(150px, 1fr));
    gap: var(--s-4);
    margin: 0;
  }

  .counters dd {
    margin: var(--s-1) 0 0;
    font-size: var(--fs-md);
  }

  .counters .big {
    font-family: var(--font-display);
    font-weight: var(--fw-numeral);
    font-size: var(--fs-xl);
  }

  .skipped summary {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    cursor: pointer;
    color: var(--text-muted);
    font-size: var(--fs-sm);
  }

  .skipped ul {
    margin-top: var(--s-3);
    display: grid;
    gap: var(--s-2);
    font-size: var(--fs-sm);
  }

  @media (min-width: 1100px) {
    .status {
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
      align-items: start;
    }

    .skipped {
      grid-column: 1 / -1;
    }
  }

  @media (max-width: 640px) {
    .screen {
      overflow: auto;
    }

    .body {
      grid-template-columns: minmax(0, 1fr);
    }

    .panel {
      border-right: 0;
      border-bottom: 1px solid var(--line);
    }

    .content {
      overflow: visible;
    }
  }
</style>
