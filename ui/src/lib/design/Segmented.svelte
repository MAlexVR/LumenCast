<script lang="ts" generics="T extends string">
  let {
    value,
    options,
    label,
    onchange,
  }: {
    value: T;
    options: { value: T; label: string }[];
    label: string;
    onchange: (next: T) => void;
  } = $props();
</script>

<div class="segmented" role="radiogroup" aria-label={label}>
  {#each options as option (option.value)}
    <button
      type="button"
      role="radio"
      aria-checked={value === option.value}
      class:active={value === option.value}
      onclick={() => onchange(option.value)}
    >
      {option.label}
    </button>
  {/each}
</div>

<style>
  .segmented {
    display: grid;
    grid-auto-flow: column;
    grid-auto-columns: 1fr;
    padding: 3px;
    gap: 3px;
    background: var(--surface-2);
    border: 1px solid var(--line);
    border-radius: var(--r-2);
  }

  button {
    height: 36px;
    border-radius: calc(var(--r-2) - 2px);
    font-size: var(--fs-sm);
    font-weight: 550;
    color: var(--text-muted);
    transition:
      background-color var(--dur-2) var(--ease-out),
      color var(--dur-2) var(--ease-out);
  }

  button:hover {
    color: var(--text);
  }

  button.active {
    background: var(--lumen);
    color: var(--on-lumen);
  }

  button:focus-visible {
    outline: 2px solid var(--lumen);
    outline-offset: 2px;
  }
</style>
