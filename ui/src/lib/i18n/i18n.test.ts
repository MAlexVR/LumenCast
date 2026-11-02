import { describe, expect, it } from "vitest";
import { en } from "./en";
import { es } from "./es";

describe("i18n dictionaries", () => {
  it("define the same keys in both languages with no empty text", () => {
    expect(Object.keys(en).sort()).toEqual(Object.keys(es).sort());
    for (const dict of [es, en] as Record<string, string>[]) {
      for (const [key, text] of Object.entries(dict)) expect(text.trim().length, key).toBeGreaterThan(0);
    }
  });
});
