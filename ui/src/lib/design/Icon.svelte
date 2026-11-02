<script lang="ts" module>
  // Hand-drawn 24px icon set (same drawing rules as LumenGet): stroke 1.5, mitre joins.
  const PATHS = {
    play: "M7.5 5v14L19 12z",
    stop: "M7 7h10v10H7z",
    restart: "M4.5 12a7.5 7.5 0 1 0 2.2-5.3M4.5 4.5v3.2h3.2",
    check: "M5 12.5 9.5 17 19 7.5",
    alert: "M12 4 21 19.5H3zM12 10v4.5M12 16.5v1",
    chevronDown: "M5.5 9.5 12 16l6.5-6.5",
    arrowUp: "M12 19.5v-15M6 10.5l6-6 6 6",
    arrowDown: "M12 4.5v15M6 13.5l6 6 6-6",
    arrowLeft: "M19.5 12h-15M10.5 6l-6 6 6 6",
    arrowRight: "M4.5 12h15M13.5 6l6 6-6 6",
    camera: "M3.5 7.5h4l1.5-2.5h6l1.5 2.5h4v11h-17zM12 10.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7z",
    effects:
      "M12 3.5l1.8 5.2 5.2 1.8-5.2 1.8L12 17.5l-1.8-5.2L5 10.5l5.2-1.8zM18.5 15.5l.8 2.2 2.2.8-2.2.8-.8 2.2-.8-2.2-2.2-.8 2.2-.8z",
    framing: "M7 3.5v13h13.5M3.5 7H17v13.5",
    audio: "M9 4.5h6v9a3 3 0 0 1-6 0zM5.5 11.5a6.5 6.5 0 0 0 13 0M12 18v2.5M8.5 20.5h7",
    settings: "M4 7h9M17 7h3M4 17h3M11 17h9M15 4.5v5M9 14.5v5",
    center: "M12 9.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z",
  } as const;
  export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
  let { name, size = 18 }: { name: IconName; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="1.5"
  stroke-linejoin="miter"
  aria-hidden="true"
  focusable="false"
>
  <path d={PATHS[name]} />
</svg>
