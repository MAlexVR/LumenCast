<script lang="ts">
  import { t } from "../i18n/index.svelte";
  import type { ImportReport } from "../types";

  let { report, ondismiss }: { report: ImportReport; ondismiss: () => void } = $props();
</script>

<aside class="notice" aria-labelledby="import-title">
  <div class="text">
    <h2 id="import-title">{t("import_title")}</h2>
    <p class="summary">
      {t("import_summary", {
        applied: report.applied.length,
        later: report.keptForLater.length,
        ignored: report.ignored.length,
      })}
    </p>
    {#if report.ignored.length > 0}
      <details>
        <summary>{t("import_details")}</summary>
        <p class="eyebrow">{t("import_ignored_title")}</p>
        <ul>
          {#each report.ignored as entry (entry.key)}
            <li><span class="num">{entry.key}</span> {entry.reason}</li>
          {/each}
        </ul>
      </details>
    {/if}
  </div>
  <button class="btn btn-quiet" type="button" onclick={ondismiss}>{t("import_dismiss")}</button>
</aside>

<style>
  .notice {
    display: flex;
    gap: var(--s-4);
    align-items: flex-start;
    justify-content: space-between;
    padding: var(--s-4) var(--s-5);
    border: 1px solid var(--line);
    border-left: 2px solid var(--lumen);
    border-radius: var(--r-2);
    background: var(--surface-1);
  }

  h2 {
    font-size: var(--fs-base);
  }

  .summary {
    margin-top: var(--s-1);
    font-size: var(--fs-sm);
    color: var(--text-muted);
  }

  details {
    margin-top: var(--s-2);
    font-size: var(--fs-sm);
  }

  summary {
    cursor: pointer;
    color: var(--text-muted);
  }

  ul {
    display: grid;
    gap: var(--s-1);
    margin-top: var(--s-2);
  }
</style>
