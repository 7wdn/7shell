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
import {
  DEFAULT_TERMINAL_FONT,
  DEFAULT_UI_FONT,
  FONT_PRESETS,
  SYSTEM_UI_FONT,
  UI_SCALE_PERCENTS,
  type CustomColors,
  type ThemePalette,
} from "@/types/app";

export type SettingsCategoryId = "appearance" | "workspace" | "files" | "security" | "about";

/** The categories, in the order the navigation draws them. */
export const SETTINGS_CATEGORIES: readonly SettingsCategoryId[] = [
  "appearance",
  "workspace",
  "files",
  "security",
  "about",
];

/**
 * The categories that are a list of something rather than a set of choices.
 *
 * Their rows are not preferences — nobody sets a trusted key to a value — so they draw their own
 * content and the search does not look inside them.
 */
export const LIST_CATEGORIES: readonly SettingsCategoryId[] = ["security"];

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
  security: "settings.nav.security",
  about: "settings.nav.about",
};

/**
 * The fields a row can write.
 *
 * Six settings have no row of their own. The file panel's height and the sidebar's width are
 * dividers somebody drags; whether the sidebar is open is a button in the sidebar's own header; and
 * the two hand-picked colour sets are five pickers that appear inside the colour-theme row once it
 * says `custom` — a row of their own would be a row that means nothing until another row is set.
 * They are reset with the category they belong to — `SettingsPage.fieldsOf` is where that is said —
 * rather than listed here.
 */
export type SettingsRowField = Exclude<
  PreferenceField,
  "terminalPanelHeight" | "sidebarWidth" | "sidebarOpen" | "customColors" | "customColorsDark"
>;

/** The glyphs a choice may carry. A closed set, because the control has to know how to draw one. */
export type SettingsChoiceIcon = "sun" | "moon" | "sun-moon";

/**
 * One of a small set of answers, drawn as a button.
 *
 * For the three settings that are modes rather than values — which theme, which terminal palette —
 * because there are only ever three of them and they are worth one click each. A dropdown hides the
 * two that were not chosen behind a click, which for "light, dark or follow the system" is a click
 * to see something people switch between all day.
 */
export interface SettingsChoiceOption {
  /** What the choice is worth, in the settings file's own spelling. */
  readonly value: string;
  readonly labelKey: string;
  /** The glyph before the word, when a shape says it faster than the word does. */
  readonly icon?: SettingsChoiceIcon;
  /**
   * A corner for the button itself, for a choice that is about roundness.
   *
   * The button then shows what it stands for rather than naming it: "large" and "small" are two
   * words for a difference the eye reads instantly off the buttons themselves.
   */
  readonly radius?: string;
}

/**
 * One card in a choice of a few, each with a line under its name.
 *
 * For the choices that are too big to be a button: a whole page's contents, a layout, a shape — the
 * name says which one it is and the line under it says what it does, because two names like
 * "overview" and "introduction" are not enough to choose between at a glance.
 */
export interface SettingsCardOption {
  /** What the choice is worth, in the settings file's own spelling. */
  readonly value: string;
  readonly titleKey: string;
  /** One line under the title: what this one does, said short enough to sit in a card. */
  readonly descriptionKey: string;
}

export interface SettingsSelectOption {
  /** What the choice is worth, in the settings file's own spelling. */
  readonly value: string;
  readonly labelKey: string;
  /** What the label needs filled in — a percentage, for instance. */
  readonly params?: Readonly<Record<string, string | number>>;
  /**
   * A colour to draw beside the label, for a choice that is a colour.
   *
   * The value is the palette's own accent in its light theme, which is what the swatch is for:
   * telling six otherwise-similar words apart at a glance.
   */
  readonly swatch?: string;
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
    | { readonly kind: "select"; readonly options: readonly SettingsSelectOption[] }
    | { readonly kind: "choice"; readonly options: readonly SettingsChoiceOption[] }
    | { readonly kind: "cards"; readonly options: readonly SettingsCardOption[] }
    /**
     * A font, which is not a dropdown of fixed choices: the options are this machine's families and
     * they are read when the list opens, and a name that is not among them can be typed and taken.
     * Which of the two fonts it is decides the rest — the families offered and the label on the
     * default — so that is all the table has to say.
     */
    | { readonly kind: "font"; readonly family: "interface" | "terminal" };
  /**
   * Drawn as a bare labelled control in the grid at the top of its category, rather than as a card
   * of its own.
   *
   * The three that carry it are the ones nobody reads a paragraph about before touching: a language,
   * a colour and a size are each a word and a control. Everything a card adds — the summary, the
   * explanation behind the question mark — is still in the table, which is what the search and the
   * tooltip go on using.
   */
  readonly presentation?: "field";
  /**
   * Which grid a field is drawn in.
   *
   * The appearance page opens with a row of three — a language, a colour and a size, each a word and
   * a control — and then a row of two for the fonts. The fonts have their own row because a row of
   * three with one column empty in it reads as a mistake, and because a family name is longer than
   * the words beside it: the picker that opens from it is a list of names.
   *
   * `wide` is a field on a line of its own, and it is for a control that is a block rather than a
   * word: cards that describe what they do are two hundred pixels each, and a row that puts them
   * beside a title squeezes the title to nothing.
   *
   * `four` is the row the tab bar's own choices are drawn in — where it sits, how it groups, how it
   * orders and how wide it draws. Four answers to one question, so they are read together rather than
   * one per screenful, and each carries its one-line summary underneath: at that width a bare control
   * is a value with nothing to say what it is for. Four across only where this page is wide enough
   * for four — measured against the page rather than against the window, because the panels beside it
   * take a share of the window that changes with their own settings. Below that they pair up: a
   * control too narrow to draw its own value is worse than a row that takes twice the height.
   */
  readonly grid?: "settings" | "fonts" | "wide" | "four";
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

/**
 * What each palette is called, and the dot drawn beside the name.
 *
 * Keyed by the wire type, so a palette added on the Rust side fails to compile here until it has a
 * name in both languages — which is the check that would otherwise have to be a test.
 */
const PALETTE_LABEL_KEYS: Record<ThemePalette, string> = {
  default: "settings.palette.default",
  mist: "settings.palette.mist",
  graphite: "settings.palette.graphite",
  cobalt: "settings.palette.cobalt",
  sage: "settings.palette.sage",
  amber: "settings.palette.amber",
  blush: "settings.palette.blush",
  vscode: "settings.palette.vscode",
  idea: "settings.palette.idea",
  xcode: "settings.palette.xcode",
  jetbrains: "settings.palette.jetbrains",
  cursor: "settings.palette.cursor",
  claude: "settings.palette.claude",
  custom: "settings.palette.custom",
};

/**
 * The dot drawn beside each name.
 *
 * The palettes' own dots, as the reference draws them: for the plain palettes that is the colour of
 * the page each one makes — near-white, then a grey, then a series of tints — and for the ones named
 * after an editor it is that editor's own accent, because there is no mistaking those for anything
 * else. A dot taken from the palette's accent instead would show six near-identical blues for the
 * six tints, which is the one thing the dot is for.
 */
const PALETTE_SWATCHES: Record<ThemePalette, string> = {
  default: "#ffffff",
  mist: "#e4eaf2",
  graphite: "#d8dce4",
  cobalt: "#d8e6f7",
  sage: "#dbe9e2",
  amber: "#f4e4b8",
  blush: "#f4d9e6",
  vscode: "#007acc",
  idea: "#4b6eaf",
  xcode: "#0a84ff",
  jetbrains: "#7b61ff",
  cursor: "#6ba4ff",
  claude: "#c47a50",
  custom: "#171717",
};

/**
 * The palettes, in the order the dropdown lists them: the order the names above are declared in.
 *
 * Read off the record rather than written out again, so that a palette added to the wire type has
 * exactly one place to be given a name — and cannot be given a name and then left out of the list.
 */
const PALETTES = Object.keys(PALETTE_LABEL_KEYS) as readonly ThemePalette[];

/**
 * What the families that carry a name of their own are called, keyed by the stack they are stored as.
 *
 * The three that are not a family at all — the two defaults and the system's own face — are here for
 * the same reason the plain ones are: so that no name is put together in a component, and every one
 * of them is a translation that exists in both languages. A family this machine has is its own name
 * and has no key; it is shown as the system spells it.
 */
export const FONT_LABEL_KEYS: Record<string, string> = {
  [SYSTEM_UI_FONT]: "settings.font.systemDefault",
  [DEFAULT_UI_FONT]: "settings.font.appDefault",
  [DEFAULT_TERMINAL_FONT]: "settings.font.appDefault",
  ...Object.fromEntries(FONT_PRESETS.map((preset) => [preset.value, preset.labelKey])),
};

/**
 * The five colours a hand-picked palette is made of, as the labels of the pickers that set them.
 *
 * The keys are the wire type's, so a colour added to a custom palette fails to compile until it has
 * a name in both languages.
 */
export const CUSTOM_COLOR_LABEL_KEYS: Record<keyof CustomColors, string> = {
  background: "settings.custom.background",
  foreground: "settings.custom.foreground",
  primary: "settings.custom.primary",
  border: "settings.custom.border",
  card: "settings.custom.card",
};

export const SETTINGS_ROWS: readonly SettingsRowDefinition[] = [
  {
    id: "language",
    category: "appearance",
    field: "locale",
    titleKey: "settings.rows.language.title",
    summaryKey: "settings.rows.language.summary",
    helpKey: "settings.rows.language.help",
    presentation: "field",
    control: {
      kind: "select",
      options: SUPPORTED_LOCALES.map((locale) => ({ value: locale, labelKey: LANGUAGE_LABEL_KEYS[locale] })),
    },
  },
  {
    id: "color-theme",
    category: "appearance",
    field: "themePalette",
    titleKey: "settings.rows.colorTheme.title",
    summaryKey: "settings.rows.colorTheme.summary",
    helpKey: "settings.rows.colorTheme.help",
    presentation: "field",
    control: {
      kind: "select",
      options: PALETTES.map((palette) => ({
        value: palette,
        labelKey: PALETTE_LABEL_KEYS[palette],
        swatch: PALETTE_SWATCHES[palette],
      })),
    },
  },
  {
    id: "ui-scale",
    category: "appearance",
    field: "uiScale",
    titleKey: "settings.rows.uiScale.title",
    summaryKey: "settings.rows.uiScale.summary",
    helpKey: "settings.rows.uiScale.help",
    presentation: "field",
    control: {
      kind: "select",
      options: UI_SCALE_PERCENTS.map((percent) => ({
        value: String(percent),
        labelKey: "settings.uiScale.percent",
        params: { percent },
      })),
    },
  },
  {
    id: "interface-font",
    category: "appearance",
    field: "uiFont",
    titleKey: "settings.rows.uiFont.title",
    summaryKey: "settings.rows.uiFont.summary",
    helpKey: "settings.rows.uiFont.help",
    presentation: "field",
    grid: "fonts",
    control: { kind: "font", family: "interface" },
  },
  {
    id: "terminal-font",
    category: "appearance",
    field: "terminalFont",
    titleKey: "settings.rows.terminalFont.title",
    summaryKey: "settings.rows.terminalFont.summary",
    helpKey: "settings.rows.terminalFont.help",
    presentation: "field",
    grid: "fonts",
    control: { kind: "font", family: "terminal" },
  },
  {
    id: "theme-mode",
    category: "appearance",
    field: "themeMode",
    titleKey: "settings.rows.themeMode.title",
    summaryKey: "settings.rows.themeMode.summary",
    helpKey: "settings.rows.themeMode.help",
    control: {
      kind: "choice",
      // Light, dark, and then the system: the two somebody switches between are first, and the one
      // that hands the decision back is last — which is also the order the icons read in.
      options: [
        { value: "light", labelKey: "settings.theme.light", icon: "sun" },
        { value: "dark", labelKey: "settings.theme.dark", icon: "moon" },
        { value: "system", labelKey: "settings.theme.system", icon: "sun-moon" },
      ],
    },
  },
  {
    id: "corner-style",
    category: "appearance",
    field: "cornerStyle",
    titleKey: "settings.rows.cornerStyle.title",
    summaryKey: "settings.rows.cornerStyle.summary",
    helpKey: "settings.rows.cornerStyle.help",
    control: {
      kind: "choice",
      // Square first, then the two that are rounded — the same order as the theme's, where what
      // changes least comes first. The values are the radii the stylesheet declares for each step.
      options: [
        { value: "none", labelKey: "settings.corner.none", radius: "0px" },
        { value: "small", labelKey: "settings.corner.small", radius: "0.375rem" },
        { value: "large", labelKey: "settings.corner.large", radius: "0.625rem" },
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
      kind: "choice",
      options: [
        { value: "light", labelKey: "settings.terminal.light", icon: "sun" },
        { value: "dark", labelKey: "settings.terminal.dark", icon: "moon" },
        { value: "system", labelKey: "settings.terminal.follow", icon: "sun-moon" },
      ],
    },
  },
  {
    id: "home-content",
    category: "appearance",
    field: "homeLayout",
    titleKey: "settings.rows.homeContent.title",
    summaryKey: "settings.rows.homeContent.summary",
    helpKey: "settings.rows.homeContent.help",
    presentation: "field",
    grid: "wide",
    control: {
      kind: "cards",
      options: [
        { value: "intro", titleKey: "settings.home.layouts.intro", descriptionKey: "settings.home.layouts.introHint" },
        {
          value: "workspace",
          titleKey: "settings.home.layouts.workspace",
          descriptionKey: "settings.home.layouts.workspaceHint",
        },
      ],
    },
  },
  {
    id: "app-layout",
    category: "appearance",
    field: "appLayout",
    titleKey: "settings.rows.appLayout.title",
    summaryKey: "settings.rows.appLayout.summary",
    helpKey: "settings.rows.appLayout.help",
    presentation: "field",
    grid: "wide",
    control: {
      kind: "cards",
      options: [
        {
          value: "separated",
          titleKey: "settings.appLayout.separated",
          descriptionKey: "settings.appLayout.separatedHint",
        },
        {
          value: "classic",
          titleKey: "settings.appLayout.classic",
          descriptionKey: "settings.appLayout.classicHint",
        },
      ],
    },
  },
  {
    id: "tab-layout",
    category: "appearance",
    field: "tabLayout",
    titleKey: "settings.rows.tabLayout.title",
    summaryKey: "settings.rows.tabLayout.summary",
    helpKey: "settings.rows.tabLayout.help",
    presentation: "field",
    grid: "wide",
    control: {
      kind: "cards",
      options: [
        {
          value: "scroll",
          titleKey: "settings.tabLayout.scroll",
          descriptionKey: "settings.tabLayout.scrollHint",
        },
        { value: "wrap", titleKey: "settings.tabLayout.wrap", descriptionKey: "settings.tabLayout.wrapHint" },
      ],
    },
  },
  {
    id: "tab-placement",
    category: "appearance",
    field: "tabPlacement",
    titleKey: "settings.rows.tabPlacement.title",
    summaryKey: "settings.rows.tabPlacement.summary",
    helpKey: "settings.rows.tabPlacement.help",
    presentation: "field",
    grid: "four",
    control: {
      kind: "select",
      options: [
        { value: "top", labelKey: "settings.tabPlacement.top" },
        { value: "bottom", labelKey: "settings.tabPlacement.bottom" },
        { value: "left", labelKey: "settings.tabPlacement.left" },
        { value: "right", labelKey: "settings.tabPlacement.right" },
      ],
    },
  },
  {
    id: "tab-group-mode",
    category: "appearance",
    field: "tabGroupMode",
    titleKey: "settings.rows.tabGroupMode.title",
    summaryKey: "settings.rows.tabGroupMode.summary",
    helpKey: "settings.rows.tabGroupMode.help",
    presentation: "field",
    grid: "four",
    control: {
      kind: "select",
      options: [
        { value: "none", labelKey: "settings.tabGroupMode.none" },
        { value: "session", labelKey: "settings.tabGroupMode.session" },
      ],
    },
  },
  {
    id: "tab-sort-mode",
    category: "appearance",
    field: "tabSortMode",
    titleKey: "settings.rows.tabSortMode.title",
    summaryKey: "settings.rows.tabSortMode.summary",
    helpKey: "settings.rows.tabSortMode.help",
    presentation: "field",
    grid: "four",
    control: {
      kind: "select",
      options: [
        { value: "manual", labelKey: "settings.tabSortMode.manual" },
        { value: "created-asc", labelKey: "settings.tabSortMode.created" },
        { value: "title-asc", labelKey: "settings.tabSortMode.title" },
      ],
    },
  },
  {
    id: "tab-max-width",
    category: "appearance",
    field: "tabMaxWidth",
    titleKey: "settings.rows.tabMaxWidth.title",
    summaryKey: "settings.rows.tabMaxWidth.summary",
    helpKey: "settings.rows.tabMaxWidth.help",
    presentation: "field",
    grid: "four",
    control: {
      kind: "select",
      options: [
        { value: "unlimited", labelKey: "settings.tabMaxWidth.unlimited" },
        { value: "wide", labelKey: "settings.tabMaxWidth.wide" },
        { value: "medium", labelKey: "settings.tabMaxWidth.medium" },
        { value: "narrow", labelKey: "settings.tabMaxWidth.narrow" },
      ],
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
 * The rows of a category that are drawn as fields, and the ones that are drawn as cards.
 *
 * A category draws its fields first, in one grid, and its cards under them — which is why the two
 * are asked for separately rather than the page deciding row by row where each one goes.
 */
export function fieldRowsOfCategory(category: SettingsCategoryId): readonly SettingsRowDefinition[] {
  return rowsOfCategory(category).filter((row) => row.presentation === "field");
}

/** The rest of a category's rows: the ones that get a card of their own. */
export function cardRowsOfCategory(category: SettingsCategoryId): readonly SettingsRowDefinition[] {
  return rowsOfCategory(category).filter((row) => row.presentation !== "field");
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
