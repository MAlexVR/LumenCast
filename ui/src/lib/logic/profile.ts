/** `lumencast-profile-2026-11-02.json`: the name a saved profile is offered under. */
export function profileFileName(date: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `lumencast-profile-${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}.json`;
}

/** How many seconds the "click again to confirm" of the reset stays open. */
export const RESET_CONFIRM_SECONDS = 4;
