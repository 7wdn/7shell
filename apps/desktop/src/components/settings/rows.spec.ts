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
  CUSTOM_COLOR_LABEL_KEYS,
  FONT_LABEL_KEYS,
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
  if (row.control.kind === "select" || row.control.kind === "choice") {
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

  it("names every colour a hand-picked palette is made of, in every language", () => {
    const missing: string[] = [];
    // Nothing here comes from a row: the five pickers appear inside the colour-theme row once it
    // says `custom`, and the heading above them is built from a key and a theme name. Neither is a
    // literal `t("…")` a checker can read, so this is where they are held to being translated.
    const keys = [
      ...Object.values(CUSTOM_COLOR_LABEL_KEYS),
      "settings.custom.reset",
      "settings.custom.editingLight",
      "settings.custom.editingDark",
    ];
    for (const language of languages) {
      const translations = messages(language);
      for (const key of keys) {
        if (!translations.has(key)) {
          missing.push(`${language}: ${key}`);
        }
      }
    }
    expect(missing).toEqual([]);
  });

  it("names the fonts that have a name of their own, in every language", () => {
    const missing: string[] = [];
    // The list is drawn from stacks — three that mean something rather than name a family, and the
    // presets — and none of those labels is a literal `t("…")` a checker can read, because they
    // arrive through the table the picker is built from.
    for (const language of languages) {
      const translations = messages(language);
      for (const key of Object.values(FONT_LABEL_KEYS)) {
        if (!translations.has(key)) {
          missing.push(`${language}: ${key}`);
        }
      }
      for (const key of ["settings.font.loading", "settings.font.empty", "settings.font.use"]) {
        if (!translations.has(key)) {
          missing.push(`${language}: ${key}`);
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

/**
 * The colour themes, against the stylesheet that draws them.
 *
 * A palette the list offers and the stylesheet does not know is a choice that does nothing: the
 * attribute is set, no block matches, and the window wears the default — which reads as "this option
 * is broken" rather than as "the CSS is missing". The two files are the guard's whole subject, so
 * both halves are read here rather than one being trusted.
 *
 * The second half is the rule `styles/globals.css` states in its own comment and cannot enforce:
 * `[data-palette="…"]` comes before `.dark` in source order, so a dark block that set fewer tokens
 * than its light one would quietly keep the light value — a white border across a dark window, in
 * whichever palettes were affected, and only visible if somebody looked at both themes in both.
 */
describe("the colour themes the page offers", () => {
  const STYLESHEET = readFileSync(path.resolve(LOCALES, "../../styles/globals.css"), "utf8");

  const themes = SETTINGS_ROWS.find((row) => row.id === "color-theme");
  if (!themes || themes.control.kind !== "select") {
    throw new Error("the table has no colour-theme row to check");
  }
  const offered = themes.control.options;

  /** The body of one palette block, or null when the stylesheet has no such block. */
  function block(palette: string, suffix: string): string | null {
    const start = STYLESHEET.indexOf(`[data-palette="${palette}"]${suffix} {`);
    if (start < 0) {
      return null;
    }
    return STYLESHEET.slice(start, STYLESHEET.indexOf("\n}", start));
  }

  /** The tokens a block sets, which is what has to match between the two themes. */
  function tokens(body: string): string[] {
    return [...body.matchAll(/^\s*(--[a-z-]+):/gm)].map((match) => match[1] as string).sort();
  }

  /** A palette the stylesheet is not expected to draw: see the case below. */
  const WITHOUT_A_BLOCK = ["default", "custom"];

  it("draws every palette it offers, in both themes", () => {
    const missing: string[] = [];
    for (const option of offered) {
      // Two palettes have no block here, for two different reasons. The default is `:root` itself.
      // A custom palette is five colours somebody picked a moment ago: it cannot be in a stylesheet
      // at all, and the store writes it as custom properties on the document instead.
      if (WITHOUT_A_BLOCK.includes(option.value)) {
        continue;
      }
      for (const suffix of ["", ".dark"]) {
        if (block(option.value, suffix) === null) {
          missing.push(`${option.value}${suffix}`);
        }
      }
    }
    expect(missing, "a palette with no block is a choice that looks broken").toEqual([]);
  });

  it("sets the same tokens in both of a palette's blocks", () => {
    const lopsided: string[] = [];
    for (const option of offered) {
      if (WITHOUT_A_BLOCK.includes(option.value)) {
        continue;
      }
      const light = block(option.value, "");
      const dark = block(option.value, ".dark");
      if (!light || !dark) {
        continue;
      }
      // The pane's own tokens are excepted, and they are the reason this case is written at all:
      // they are named by theme (`--terminal-dark-…`, `--terminal-light-…`) and both halves are set
      // in one block, because a dark pane beside a light window still has to wear the palette.
      const themed = (body: string): string =>
        tokens(body)
          .filter((name) => !name.startsWith("--terminal-"))
          .join(" ");
      if (themed(light) !== themed(dark)) {
        lopsided.push(option.value);
      }
    }
    expect(lopsided, "a token set in one theme and not the other leaks between them").toEqual([]);
  });

  it("dresses the terminal from every palette, in both of its own themes", () => {
    const missing: string[] = [];
    // A custom palette is five colours the store writes as custom properties, and
    // `customColors.spec.ts` is where that half is held. Everything else is here — the base palette
    // included, and it is the case worth having: it has no block of its own, so its tokens are
    // `:root` itself, which is exactly where somebody adding a palette would forget to look.
    const plain = (selector: string): string | null => {
      const start = STYLESHEET.indexOf(`${selector} {`);
      return start < 0 ? null : STYLESHEET.slice(start, STYLESHEET.indexOf("\n}", start));
    };
    for (const option of offered) {
      const body = option.value === "default" ? plain(":root") : block(option.value, "");
      if (body === null) {
        continue;
      }
      const names = tokens(body);
      for (const theme of ["dark", "light"]) {
        for (const part of ["background", "foreground", "cursor", "selection"]) {
          if (!names.includes(`--terminal-${theme}-${part}`)) {
            missing.push(`${option.value}: --terminal-${theme}-${part}`);
          }
        }
      }
    }
    expect(missing, "a palette the terminal cannot wear leaves the pane in the build's own greys").toEqual([]);
  });

  it("gives every palette a dot, so the list says what it is before it is worn", () => {
    const missing = offered.filter((option) => !option.swatch).map((option) => option.value);
    expect(missing, "a colour with no swatch is one you have to choose to see").toEqual([]);
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
