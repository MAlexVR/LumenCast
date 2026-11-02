<script lang="ts">
  import { t } from "../i18n/index.svelte";

  let {
    checked,
    label,
    hint = "",
    disabled = false,
    badge = "",
    onchange,
  }: {
    checked: boolean;
    label: string;
    hint?: string;
    disabled?: boolean;
    badge?: string;
    onchange: (next: boolean) => void;
  } = $props();
</script>

<label class="switch" class:disabled>
  <span class="text">
    <span class="strong">{label}{#if badge}<span class="badge">{badge}</span>{/if}</span>
    {#if hint}<span class="hint">{hint}</span>{/if}
  </span>
  <input type="checkbox" role="switch" {checked} {disabled} onchange={(e) => onchange(e.currentTarget.checked)} />
  <span class="track" aria-hidden="true">
    <span class="word">{checked ? t("switch_on") : t("switch_off")}</span>
    <span class="knob"></span>
  </span>
</label>

<style>
  .switch {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s-4);
    cursor: pointer;
  }

  .switch.disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }

  .text {
    display: grid;
    gap: var(--s-1);
    min-width: 0;
  }

  .strong {
    font-size: var(--fs-sm);
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--s-2);
  }

  .badge {
    white-space: nowrap;
    font-size: var(--fs-xs);
    color: var(--text-muted);
    border: 1px solid var(--line);
    border-radius: 999px;
    padding: 0 var(--s-2);
  }

  .hint {
    font-size: var(--fs-xs);
    color: var(--text-muted);
  }

  input {
    position: absolute;
    opacity: 0;
    pointer-events: none;
  }

  .track {
    position: relative;
    flex: none;
    width: 64px;
    height: 28px;
    border-radius: 999px;
    background: var(--surface-3);
    border: 1px solid var(--line);
    display: flex;
    align-items: center;
    transition: background-color var(--dur-2) var(--ease-out);
  }

  .word {
    flex: 1;
    text-align: center;
    font-size: 11px;
    font-weight: 650;
    letter-spacing: 0.06em;
    color: var(--text-muted);
    padding: 0 6px 0 28px;
  }

  .knob {
    position: absolute;
    left: 3px;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--text);
    transition: transform var(--dur-2) var(--ease-out);
  }

  input:checked + .track {
    background: var(--lumen);
    border-color: var(--lumen);
  }

  input:checked + .track .word {
    color: var(--on-lumen);
    padding: 0 28px 0 6px;
  }

  input:checked + .track .knob {
    transform: translateX(36px);
    background: var(--on-lumen);
  }

  input:focus-visible + .track {
    outline: 2px solid var(--lumen);
    outline-offset: 2px;
  }
</style>
