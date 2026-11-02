<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import { errorMessageKey } from "../logic/errors";
  import { formatSize, installKindKey, phaseKey, progressPercent } from "../logic/updates";
  import type { ErrorPayload, UpdateProgress, UpdateStatus } from "../types";
  import Icon from "./Icon.svelte";

  let {
    status,
    version,
    checking,
    progress,
    installing,
    error,
    oncheck,
    oninstall,
    oncancel,
  }: {
    status: UpdateStatus | null;
    version: string;
    checking: boolean;
    progress: UpdateProgress | null;
    installing: boolean;
    error: ErrorPayload | null;
    oncheck: () => void;
    oninstall: () => void;
    oncancel: () => void;
  } = $props();

  const available = $derived(status?.available ?? null);
  const percent = $derived(progress ? progressPercent(progress) : null);
  const size = $derived(available ? formatSize(available.size) : "");
</script>

<section class="updates" aria-label={t("update_title")}>
  <h2 class="eyebrow">{t("update_title")}</h2>
  <p class="hint num">{t("update_current", { version: status?.current ?? version })}</p>
  {#if status}
    <p class="hint">{t(installKindKey(status.install))}</p>
  {/if}

  {#if !installing}
    <button class="btn btn-quiet" type="button" onclick={oncheck} disabled={checking}>
      <Icon name="restart" />{checking ? t("update_checking") : t("update_check")}
    </button>
  {/if}

  {#if status && !available && !error}
    <p class="note" role="status"><Icon name="check" size={16} />{t("update_up_to_date")}</p>
  {/if}

  {#if available}
    <div class="available" role="status">
      <p class="title">{t("update_available", { version: available.version })}</p>
      {#if size}<p class="hint num">{t("update_size", { size })}</p>{/if}
      {#if available.notes}
        <details>
          <summary>{t("update_notes")}</summary>
          <pre class="notes">{available.notes}</pre>
        </details>
      {/if}
      {#if available.page}
        <p class="hint"><span class="muted">{t("update_release_page")}:</span> <span class="num">{available.page}</span></p>
      {/if}
      {#if installing}
        <p class="hint" aria-live="polite">{progress ? t(phaseKey(progress.phase)) : t("update_downloading")}</p>
        <progress max="100" value={percent ?? undefined} aria-label={t("update_downloading")}></progress>
        {#if progress?.phase === "downloading"}
          <button class="btn btn-quiet" type="button" onclick={oncancel}>{t("update_cancel")}</button>
        {/if}
      {:else if available.canInstall && status?.install !== "unmanaged"}
        <button class="btn btn-primary" type="button" onclick={oninstall}><Icon name="arrowDown" />{t("update_install")}</button>
      {:else}
        <p class="hint">{t("update_manual")}</p>
      {/if}
    </div>
  {/if}

  {#if error}
    <p class="problem" role="alert"><Icon name="alert" size={16} />{t(errorMessageKey(error.code))}</p>
  {/if}
</section>

<style>
  .updates {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    min-width: 0;
    gap: var(--s-3);
    padding: var(--s-4);
    background: var(--surface-2);
    border: 1px solid var(--line-soft);
    border-radius: var(--r-2);
  }

  .hint {
    font-size: var(--fs-xs);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .muted {
    color: var(--text-muted);
  }

  .available {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    min-width: 0;
    gap: var(--s-2);
    padding: var(--s-3);
    border: 1px solid var(--line);
    border-left: 2px solid var(--lumen);
    border-radius: var(--r-2);
    background: var(--surface-1);
  }

  .title {
    font-size: var(--fs-base);
  }

  .notes {
    margin: var(--s-2) 0 0;
    max-height: 160px;
    overflow: auto;
    white-space: pre-wrap;
    font-family: inherit;
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }

  details {
    font-size: var(--fs-sm);
  }

  progress {
    width: 100%;
    accent-color: var(--lumen);
  }

  .note {
    display: flex;
    gap: var(--s-2);
    align-items: center;
    font-size: var(--fs-xs);
    color: var(--ok, var(--text-muted));
  }

  .problem {
    display: flex;
    gap: var(--s-2);
    align-items: baseline;
    font-size: var(--fs-xs);
    color: var(--danger);
  }
</style>
