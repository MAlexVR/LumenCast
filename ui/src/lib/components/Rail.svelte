<script lang="ts" generics="T extends string">
  import Icon, { type IconName } from "../design/Icon.svelte";
  import { t } from "../i18n/index.svelte";

  let {
    items,
    selected,
    version,
    onselect,
  }: {
    items: { id: T; icon: IconName; label: string }[];
    selected: T;
    version: string;
    onselect: (id: T) => void;
  } = $props();
</script>

<nav class="rail" aria-label={t("tabs_label")}>
  <div class="brand">
    <img src="/favicon.svg" width="28" height="28" alt="" aria-hidden="true" draggable="false" />
    <span class="wordmark">{t("app_title")}</span>
  </div>
  <ul>
    {#each items as item (item.id)}
      <li>
        <button
          class="item"
          type="button"
          aria-current={selected === item.id ? "page" : undefined}
          aria-controls={`panel-${item.id}`}
          title={item.label}
          onclick={() => onselect(item.id)}
        >
          <Icon name={item.icon} />
          <span class="label">{item.label}</span>
        </button>
      </li>
    {/each}
  </ul>
  <div class="foot"><span class="version num">v{version}</span></div>
</nav>

<style>
  .rail {
    display: flex;
    flex-direction: column;
    gap: var(--s-8);
    width: var(--rail-w);
    padding: var(--s-6) var(--s-3);
    border-right: 1px solid var(--line);
    background: var(--bg);
  }

  .brand {
    display: flex;
    align-items: center;
    gap: var(--s-2);
    padding: 0 var(--s-3);
    color: var(--text);
    height: 32px;
  }

  .brand img {
    display: block;
    flex: none;
  }

  .wordmark {
    font-family: var(--font-display);
    font-size: 1.3125rem;
    font-weight: var(--fw-display);
    letter-spacing: -0.02em;
    line-height: 1;
    padding-top: 2px;
  }

  ul {
    display: grid;
    gap: var(--s-1);
  }

  .item {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--s-3);
    width: 100%;
    height: 40px;
    padding: 0 var(--s-3);
    border-radius: var(--r-2);
    color: var(--text-muted);
    font-size: var(--fs-sm);
    font-weight: 500;
    text-align: left;
    transition:
      color var(--dur-1) var(--ease-out),
      background-color var(--dur-1) var(--ease-out);
  }

  .item:hover {
    color: var(--text);
    background: var(--surface-1);
  }

  .item[aria-current="page"] {
    color: var(--text);
  }

  .item[aria-current="page"]::before {
    content: "";
    position: absolute;
    left: calc(-1 * var(--s-3));
    top: 10px;
    bottom: 10px;
    width: 2px;
    background: var(--lumen);
  }

  .foot {
    margin-top: auto;
    display: grid;
    gap: var(--s-1);
    padding: 0 var(--s-3);
  }

  .version {
    font-size: var(--fs-2xs);
    color: var(--text-muted);
  }

  @media (max-width: 900px) {
    .rail {
      padding-inline: var(--s-2);
      align-items: center;
    }

    .brand {
      padding: 0;
    }

    .wordmark,
    .label,
    .foot {
      display: none;
    }

    .item {
      justify-content: center;
      padding: 0;
      width: 44px;
    }

    .item[aria-current="page"]::before {
      left: calc(-1 * var(--s-2));
    }
  }
</style>
