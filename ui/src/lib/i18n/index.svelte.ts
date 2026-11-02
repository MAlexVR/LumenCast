// Tiny typed i18n store: Spanish first, English second.
import { en } from "./en";
import { es, type MessageKey } from "./es";

export type Locale = "es" | "en";
export const LOCALES: readonly Locale[] = ["es", "en"];
const DICTS: Record<Locale, Record<MessageKey, string>> = { es, en };
const STORAGE_KEY = "lumencast.locale";

function initialLocale(): Locale {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "es" || saved === "en") return saved;
  } catch {
    /* storage unavailable: use default */
  }
  return "es";
}

export const i18n = $state<{ locale: Locale }>({ locale: initialLocale() });

export function setLocale(locale: Locale): void {
  i18n.locale = locale;
  document.documentElement.lang = locale;
  try {
    localStorage.setItem(STORAGE_KEY, locale);
  } catch {
    /* not persisted */
  }
}

export function t(key: MessageKey, vars?: Record<string, string | number>): string {
  const msg = DICTS[i18n.locale][key];
  return vars ? msg.replace(/\{(\w+)\}/g, (m, k: string) => (k in vars ? String(vars[k]) : m)) : msg;
}

export type { MessageKey };
