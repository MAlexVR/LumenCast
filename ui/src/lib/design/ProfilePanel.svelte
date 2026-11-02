<script lang="ts">
  import { onDestroy } from "svelte";
  import { t } from "../i18n/index.svelte";
  import { errorMessageKey } from "../logic/errors";
  import { RESET_CONFIRM_SECONDS } from "../logic/profile";
  import type { ErrorPayload } from "../types";
  import Icon from "./Icon.svelte";

  type Notice = { kind: "saved" | "loaded" | "reset"; file?: string } | null;

  let {
    notice,
    error,
    busy,
    onsave,
    onload,
    onreset,
  }: {
    notice: Notice;
    error: ErrorPayload | null;
    busy: boolean;
    onsave: () => void;
    onload: () => void;
    onreset: () => void;
  } = $props();

  /** The reset asks to be pressed twice: the second press, within a few seconds, does it. */
  let confirming = $state(false);
  let timer: ReturnType<typeof setTimeout> | undefined;

  function reset() {
    if (!confirming) {
      confirming = true;
      timer = setTimeout(() => (confirming = false), RESET_CONFIRM_SECONDS * 1000);
      return;
    }
    clearTimeout(timer);
    confirming = false;
    onreset();
  }

  onDestroy(() => clearTimeout(timer));

  const fileName = (path: string) => path.split(/[\\/]/).pop() || path;
</script>

<section class="profile" aria-label={t("profile_title")}>
  <h2 class="eyebrow"><Icon name="profile" size={16} />{t("profile_title")}</h2>
  <p class="hint">{t("profile_tip")}</p>

  <div class="actions">
    <button class="action" type="button" onclick={onsave} disabled={busy}>
      <span class="badge"><Icon name="save" size={20} /></span>
      <span class="text">
        <span class="name">{t("profile_save")}</span>
        <span class="hint">{t("profile_save_tip")}</span>
      </span>
    </button>
    <button class="action" type="button" onclick={onload} disabled={busy}>
      <span class="badge"><Icon name="load" size={20} /></span>
      <span class="text">
        <span class="name">{t("profile_load")}</span>
        <span class="hint">{t("profile_load_tip")}</span>
      </span>
    </button>
    <button class="action danger" class:armed={confirming} type="button" onclick={reset} disabled={busy}>
      <span class="badge"><Icon name="reset" size={20} /></span>
      <span class="text">
        <span class="name">{confirming ? t("profile_reset_confirm") : t("profile_reset")}</span>
        <span class="hint">{t("profile_reset_tip")}</span>
      </span>
    </button>
  </div>

  {#if notice && !error}
    <p class="note" role="status" aria-live="polite">
      <Icon name="check" size={16} />
      {#if notice.kind === "saved"}{t("profile_saved", { file: fileName(notice.file ?? "") })}
      {:else if notice.kind === "loaded"}{t("profile_loaded", { file: fileName(notice.file ?? "") })}
      {:else}{t("profile_was_reset")}{/if}
    </p>
  {/if}
  {#if error}
    <p class="problem" role="alert"><Icon name="alert" size={16} />{t(errorMessageKey(error.code))}</p>
  {/if}
</section>

<style>
  .profile {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    min-width: 0;
    gap: var(--s-3);
    padding: var(--s-4);
    background: var(--surface-2);
    border: 1px solid var(--line-soft);
    border-radius: var(--r-2);
  }

  .eyebrow {
    display: flex;
    align-items: center;
    gap: var(--s-2);
  }

  .hint {
    font-size: var(--fs-xs);
    color: var(--text-muted);
    overflow-wrap: anywhere;
  }

  .actions {
    display: grid;
    gap: var(--s-2);
  }

  .action {
    display: grid;
    grid-template-columns: 40px minmax(0, 1fr);
    gap: var(--s-3);
    align-items: center;
    padding: var(--s-3);
    text-align: left;
    background: var(--surface-1);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
    transition: border-color 120ms ease, background 120ms ease;
  }

  .action:hover:not(:disabled) {
    border-color: var(--lumen);
  }

  .action:disabled {
    opacity: 0.55;
  }

  .badge {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    color: var(--lumen);
    background: var(--surface-2);
    border: 1px solid var(--line-soft);
    border-radius: var(--r-2);
  }

  .text {
    display: grid;
    gap: 2px;
    min-width: 0;
  }

  .name {
    font-size: var(--fs-sm);
    color: var(--text);
  }

  .action.danger:hover:not(:disabled) {
    border-color: var(--danger);
  }

  .action.danger:hover:not(:disabled) .badge,
  .action.armed .badge {
    color: var(--danger);
  }

  .action.armed {
    border-color: var(--danger);
    background: var(--surface-2);
  }

  .note,
  .problem {
    display: flex;
    gap: var(--s-2);
    align-items: baseline;
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }

  .note {
    color: var(--ok, var(--text-muted));
  }

  .problem {
    color: var(--danger);
  }
</style>
