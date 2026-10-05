/**
 * The settings table, and the one thing the i18n gate cannot see.
 *
 * `scripts/check-i18n.mjs` reads translation keys off literal `t("…")` calls, which is exactly what
 * this page does not write: its rows name their strings through a table, so a key with a typo in it
 * would show up in the window as a raw key and no gate would fail. This spec is that gate — it
 * walks the table and checks every key against both locale files.
 *
 * The rest is what the page assumes about the table: one row per field, a row in every category
 * that has settings, and identities that are unique. Each of those is a broken page rather than a
 * broken string, and none of them is visible to the compiler.
 */
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

import {
  SETTINGS_CATEGORIES,
  LIST_CATEGORIES,
  SETTINGS_CATEGORY_LABEL_KEYS,
  SETTINGS_ROWS,
  type SettingsRowDefinition,
  rowMatches,
  rowsOfCategory,
} from "@/components/settings/rows";

const LOCALES = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../i18n/locales");

function flat(value: unknown, prefix: string, out: Map<string, string>): void {
  for (const [key, entry] of Object.entries(value as Record<string, unknown>)) {
    const full = prefix ? `${prefix}.${key}` : key;
    if (entry !== null && typeof entry === "object") {
      flat(entry, full, out);
    } else {
      out.set(full, String(entry));
    }
  }
}

/** Every string in one language, keyed the way the interface asks for it. */
function messages(language: string): Map<string, string> {
  const out = new Map<string, string>();
  const directory = path.join(LOCALES, language);
  for (const name of readdirSync(directory)) {
    if (name.endsWith(".json")) {
      flat(JSON.parse(readFileSync(path.join(directory, name), "utf8")), "", out);
    }
  }
  return out;
}

/** Every key a row asks for, whichever kind of control it carries. */
function keysOf(row: SettingsRowDefinition): string[] {
  const keys = [row.titleKey, row.summaryKey, row.helpKey];
  if (row.control.kind === "select") {
    keys.push(...row.control.options.map((option) => option.labelKey));
  }
  return keys;
}

describe("the settings table", () => {
  const languages = ["en", "zh-CN"];

  it("names every string it draws, in every language", () => {
    const missing: string[] = [];
    for (const language of languages) {
      const translations = messages(language);
      for (const row of SETTINGS_ROWS) {
        for (const key of keysOf(row)) {
          if (!translations.has(key)) {
            missing.push(`${language}: ${key} (${row.id})`);
          }
        }
      }
      for (const key of Object.values(SETTINGS_CATEGORY_LABEL_KEYS)) {
        if (!translations.has(key)) {
          missing.push(`${language}: ${key} (a category name)`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it("writes each preference field exactly once", () => {
    const fields = SETTINGS_ROWS.map((row) => row.field);
    expect(new Set(fields).size, `two rows writing one field: ${fields.join(", ")}`).toBe(fields.length);
  });

  it("gives every row an identity of its own", () => {
    const ids = SETTINGS_ROWS.map((row) => row.id);
    expect(new Set(ids).size, `two rows sharing an id: ${ids.join(", ")}`).toBe(ids.length);
  });

  it("draws a row in every category the navigation names, except the ones that are lists", () => {
    for (const category of SETTINGS_CATEGORIES) {
      // About holds what the build is — a version, a platform — rather than a setting anybody
      // chooses, so it is the one category the table does not describe.
      if (category === "about") {
        expect(rowsOfCategory(category), "About is the build's own facts").toEqual([]);
        continue;
      }
      // A list category is not a set of choices: the trusted keys are read and revoked, not set to
      // a value, so the table that describes settings has nothing to say about them.
      if (LIST_CATEGORIES.includes(category)) {
        expect(rowsOfCategory(category), `${category} is a list, not settings`).toEqual([]);
        continue;
      }
      expect(rowsOfCategory(category).length, `no rows in ${category}`).toBeGreaterThan(0);
    }
  });

  it("puts every row in a category the navigation names", () => {
    const unknown = SETTINGS_ROWS.filter((row) => !SETTINGS_CATEGORIES.includes(row.category));
    expect(unknown.map((row) => row.id)).toEqual([]);
  });
});

describe("what the search is about", () => {
  const english = messages("en");
  const text = (key: string): string => english.get(key) ?? key;
  const monitor = SETTINGS_ROWS.find((row) => row.id === "monitor");
  if (!monitor) {
    throw new Error("the table has no monitor row to search for");
  }

  it("finds a row by its title", () => {
    expect(rowMatches(monitor, "host", text)).toBe(true);
  });

  it("finds a row by its one-line summary as well", () => {
    expect(rowMatches(monitor, "PANEL", text)).toBe(true);
  });

  it("does not find a row by something neither line says", () => {
    expect(rowMatches(monitor, "shortcut", text)).toBe(false);
  });

  it("treats an empty or blank query as no search at all", () => {
    expect(rowMatches(monitor, "", text)).toBe(true);
    expect(rowMatches(monitor, "   ", text)).toBe(true);
  });
});
