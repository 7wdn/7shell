/**
 * What the settings page draws, row by row.
 *
 * The table is data rather than a copy of the same markup per setting: a row knows which field it
 * writes, which three strings it says, and which control carries it. That is what lets the page
 * render every row the same way, the search be nothing but this table filtered by text, and
 * "restore the defaults" know exactly which fields a category is made of.
 *
 * The keys are not passed through `t()` here — the translations are resolved where the row is
 * drawn — so `rows.spec.ts` walks this table and checks every key against both locale files. It is
 * the same guard `scripts/check-i18n.mjs` applies to literal `t("…")` calls, which cannot see a
 * key that arrives through a table like this one.
 */
import { SUPPORTED_LOCALES, type SupportedLocale } from "@/i18n";
import type { PreferenceField } from "@/stores/preferences";

export type SettingsCategoryId = "appearance" | "workspace" | "files" | "about";

/** The categories, in the order the navigation draws them. */
export const SETTINGS_CATEGORIES: readonly SettingsCategoryId[] = ["appearance", "workspace", "files", "about"];

/**
 * What each category is called, as a key rather than as a resolved string.
 *
 * The navigation and the section headings both read it from here, which is also what lets
 * `rows.spec.ts` check the four names against both locale files — a key built inside the template
 * (`settings.nav.${id}`) is one no guard can see.
 */
export const SETTINGS_CATEGORY_LABEL_KEYS: Record<SettingsCategoryId, string> = {
  appearance: "settings.nav.appearance",
  workspace: "settings.nav.workspace",
  files: "settings.nav.files",
  about: "settings.nav.about",
};

/**
 * The fields a row can write.
 *
 * The panel height is a setting as well, but nobody chooses it in a row: it is a divider somebody
 * drags, and it is reset with the workspace rather than listed in it.
 */
export type SettingsRowField = Exclude<PreferenceField, "terminalPanelHeight">;

export interface SettingsSelectOption {
  /** What the choice is worth, in the settings file's own spelling. */
  readonly value: string;
  readonly labelKey: string;
}

export interface SettingsRowDefinition {
  /** Stable identity: the search index, the tests and the DOM all name a row by it. */
  readonly id: string;
  readonly category: SettingsCategoryId;
  /** Which field the control writes — and what restoring this category's defaults sets back. */
  readonly field: SettingsRowField;
  readonly titleKey: string;
  /** One line under the title: what the setting is, said short enough to sit in a row. */
  readonly summaryKey: string;
  /** The whole explanation, one keystroke away behind the question mark beside the title. */
  readonly helpKey: string;
  readonly control:
    | { readonly kind: "switch" }
    | { readonly kind: "select"; readonly options: readonly SettingsSelectOption[] };
}

/**
 * What a language is called in the list of languages.
 *
 * Deliberately the same string in both locale files: somebody looking for their own language looks
 * for it written in that language, so 简体中文 stays 简体中文 in an English interface. The keys are
 * still translation keys — the rule is that no user-facing text lives in the code, not that every
 * translation happens to differ.
 */
const LANGUAGE_LABEL_KEYS: Record<SupportedLocale, string> = {
  en: "settings.language.english",
  "zh-CN": "settings.language.simplifiedChinese",
};

export const SETTINGS_ROWS: readonly SettingsRowDefinition[] = [
  {
    id: "theme-mode",
    category: "appearance",
    field: "themeMode",
    titleKey: "settings.rows.themeMode.title",
    summaryKey: "settings.rows.themeMode.summary",
    helpKey: "settings.rows.themeMode.help",
    control: {
      kind: "select",
      options: [
        { value: "system", labelKey: "settings.theme.system" },
        { value: "dark", labelKey: "settings.theme.dark" },
        { value: "light", labelKey: "settings.theme.light" },
      ],
    },
  },
  {
    id: "terminal-theme",
    category: "appearance",
    field: "terminalTheme",
    titleKey: "settings.rows.terminalTheme.title",
    summaryKey: "settings.rows.terminalTheme.summary",
    helpKey: "settings.rows.terminalTheme.help",
    control: {
      kind: "select",
      options: [
        { value: "system", labelKey: "settings.terminal.follow" },
        { value: "dark", labelKey: "settings.terminal.dark" },
        { value: "light", labelKey: "settings.terminal.light" },
      ],
    },
  },
  {
    id: "language",
    category: "appearance",
    field: "locale",
    titleKey: "settings.rows.language.title",
    summaryKey: "settings.rows.language.summary",
    helpKey: "settings.rows.language.help",
    control: {
      kind: "select",
      options: SUPPORTED_LOCALES.map((locale) => ({ value: locale, labelKey: LANGUAGE_LABEL_KEYS[locale] })),
    },
  },
  {
    id: "session-layout",
    category: "workspace",
    field: "sessionLayout",
    titleKey: "settings.rows.sessionLayout.title",
    summaryKey: "settings.rows.sessionLayout.summary",
    helpKey: "settings.rows.sessionLayout.help",
    control: {
      kind: "select",
      options: [
        { value: "split", labelKey: "settings.session.split" },
        { value: "terminal", labelKey: "settings.session.terminal" },
      ],
    },
  },
  {
    id: "monitor",
    category: "workspace",
    field: "monitorEnabled",
    titleKey: "settings.rows.monitor.title",
    summaryKey: "settings.rows.monitor.summary",
    helpKey: "settings.rows.monitor.help",
    control: { kind: "switch" },
  },
  {
    id: "files-layout",
    category: "files",
    field: "filesLayout",
    titleKey: "settings.rows.filesLayout.title",
    summaryKey: "settings.rows.filesLayout.summary",
    helpKey: "settings.rows.filesLayout.help",
    control: {
      kind: "select",
      options: [
        { value: "tree", labelKey: "settings.files.tree" },
        { value: "list", labelKey: "settings.files.list" },
      ],
    },
  },
  {
    id: "packed-transfer",
    category: "files",
    field: "packedTransfer",
    titleKey: "settings.rows.packed.title",
    summaryKey: "settings.rows.packed.summary",
    helpKey: "settings.rows.packed.help",
    control: { kind: "switch" },
  },
];

/** The rows of one category, in the order they are drawn. */
export function rowsOfCategory(category: SettingsCategoryId): readonly SettingsRowDefinition[] {
  return SETTINGS_ROWS.filter((row) => row.category === category);
}

/**
 * Whether the search is about this row.
 *
 * The title and the one-line summary are what is searched — the same two lines that are on screen,
 * so a search can never find a row for a reason the person cannot see. The empty query matches
 * everything, which is what "no search" means.
 */
export function rowMatches(row: SettingsRowDefinition, query: string, text: (key: string) => string): boolean {
  const needle = query.trim().toLowerCase();
  if (needle === "") {
    return true;
  }
  return `${text(row.titleKey)} ${text(row.summaryKey)}`.toLowerCase().includes(needle);
}
