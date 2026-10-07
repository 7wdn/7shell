/**
 * Hand-written mirror of the Rust wire types.
 *
 * `packages/app-tests/contracts.test.ts` compares every field here with its Rust counterpart,
 * in both directions, so a rename on one side fails the test suite rather than a release.
 */

/** Facts about the running application, from `sevenshell_types::AppInfo`. */
export interface AppInfo {
  readonly name: string;
  readonly version: string;
  readonly os: string;
  readonly arch: string;
  readonly debugBuild: boolean;
  readonly protocolCount: number;
  readonly descriptorSchemaVersion: number;
}

/** How the interface picks its colours, from `sevenshell_types::ThemeMode`. */
export type ThemeMode = "system" | "dark" | "light";

/** What the colours resolve to once "system" has been answered. */
export type ResolvedTheme = "dark" | "light";

/**
 * What colour the interface is made of, from `sevenshell_types::ThemePalette`.
 *
 * Not a third theme: light and dark are the theme, and either can be worn in any of these. Written
 * as one union rather than an enum so it matches the Rust side exactly, which the contract test
 * checks.
 */
export type ThemePalette =
  | "default"
  | "mist"
  | "graphite"
  | "cobalt"
  | "sage"
  | "amber"
  | "blush"
  | "vscode"
  | "idea"
  | "xcode"
  | "jetbrains"
  | "cursor"
  | "claude"
  | "custom";

/**
 * The five colours a custom palette is built from, from `sevenshell_types::CustomColors`.
 *
 * Everything else the window is made of is derived from these — see
 * `lib/settings/customColors.ts` — so there is no way to pick a combination nobody can read.
 */
export interface CustomColors {
  /** `#rrggbb`, which is what a colour input hands out and the only shape the backend keeps. */
  readonly background: string;
  readonly foreground: string;
  readonly primary: string;
  readonly border: string;
  /** The panels: the sidebar, the dialogs, the cards. */
  readonly card: string;
}

/** The five knobs, in the order the setting draws them, which is also the order they are read in. */
export const CUSTOM_COLOR_KEYS: readonly (keyof CustomColors)[] = [
  "background",
  "foreground",
  "primary",
  "border",
  "card",
];

/** What a custom palette is before anybody chooses one, in the light theme. */
export const DEFAULT_CUSTOM_COLORS: CustomColors = {
  background: "#ffffff",
  foreground: "#0a0a0a",
  primary: "#171717",
  border: "#e5e5e5",
  card: "#fafafa",
};

/** The same in the dark theme: two sets, because the two are chosen apart. */
export const DEFAULT_CUSTOM_COLORS_DARK: CustomColors = {
  background: "#131416",
  foreground: "#d7d7db",
  primary: "#d0d0d6",
  border: "#6e6e72",
  card: "#19191c",
};

/**
 * The interface scales this build offers, in percent, from `sevenshell_types::UI_SCALE_PERCENTS`.
 *
 * Both sides list them and `packages/app-tests/contracts.test.ts` compares the two lists: a scale
 * one side offers and the other refuses would be a choice that silently came back as 100.
 */
export const UI_SCALE_PERCENTS: readonly number[] = [75, 90, 100, 110, 125, 150, 175, 200];

/** The size the interface is drawn at until somebody changes it, from `ui_scale_default()`. */
export const DEFAULT_UI_SCALE = 100;

/**
 * What the interface draws its text in until somebody chooses a font.
 *
 * The same stack the stylesheet declares for `--font-sans`, and written out again here because the
 * setting has to be able to *show* what "the default" is: a picker whose first entry is a name it
 * cannot print is a picker with an empty row in it.
 */
export const DEFAULT_UI_FONT = `Inter, "Segoe UI", system-ui, sans-serif`;

/** The same for the terminal, matching the stylesheet's `--font-mono`. */
export const DEFAULT_TERMINAL_FONT = `"JetBrains Mono", Consolas, ui-monospace, monospace`;

/**
 * The interface as the operating system would draw it, which is what somebody who wants this window
 * to look like the rest of their desktop is asking for.
 */
export const SYSTEM_UI_FONT = `system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif`;

/**
 * The families offered by name, before the machine's own list.
 *
 * Each is a stack rather than a name, and each carries what follows the face it names: a font that
 * is not installed is common enough that the fallbacks are part of the choice rather than a detail
 * of how it is applied. The last one is the keyword itself — "whatever this system calls fixed
 * width" — which is a real answer for somebody who does not want to pick a face at all.
 */
export const FONT_PRESETS: readonly { readonly value: string; readonly labelKey: string }[] = [
  {
    value: "'Fira Code', 'Cascadia Code', 'Cascadia Mono', 'JetBrains Mono', monospace",
    labelKey: "settings.font.presets.firaCode",
  },
  { value: "'JetBrains Mono', 'Fira Code', monospace", labelKey: "settings.font.presets.jetbrainsMono" },
  { value: "'Cascadia Code', 'Cascadia Mono', monospace", labelKey: "settings.font.presets.cascadiaCode" },
  { value: "'Source Code Pro', monospace", labelKey: "settings.font.presets.sourceCodePro" },
  { value: "'SF Mono', 'Menlo', monospace", labelKey: "settings.font.presets.sfMono" },
  { value: "'Consolas', 'Courier New', monospace", labelKey: "settings.font.presets.consolas" },
  { value: "monospace", labelKey: "settings.font.presets.systemMonospace" },
];

/** How round the interface's corners are, from `sevenshell_types::CornerStyle`. */
export type CornerStyle = "none" | "small" | "large";

/**
 * The radius each step is worth, as the stylesheet declares it.
 *
 * Written here as well as in `styles/globals.css` because the buttons that choose a corner style are
 * drawn with the corner they stand for — the button *is* the preview — and a button cannot read a
 * token it is about to change.
 */
export const CORNER_STYLE_RADII: Record<CornerStyle, string> = {
  none: "0px",
  small: "0.375rem",
  large: "0.625rem",
};

/** What the interface is drawn with until somebody chooses otherwise: the way it has always been. */
export const DEFAULT_CORNER_STYLE: CornerStyle = "large";

/** How the file browser is laid out, from `sevenshell_types::FilesLayout`. */
export type FilesLayout = "tree" | "list";

/** How a session opens, from `sevenshell_types::SessionLayout`. */
export type SessionLayout = "terminal" | "split";

/** Non-secret preferences, from `sevenshell_types::AppSettings`. */
export interface AppSettings {
  readonly themeMode: ThemeMode;
  /** The tint worn on top of the theme: the same two themes, in different colours. */
  readonly themePalette: ThemePalette;
  /** `system` means "follow the interface theme" for the terminal, not the operating system. */
  readonly terminalTheme: ThemeMode;
  readonly filesLayout: FilesLayout;
  /** Whether opening a session opens a terminal on its own, or one with its file panel. */
  readonly sessionLayout: SessionLayout;
  /** How tall the file panel under a terminal is, in pixels. Absent means nobody has moved it. */
  readonly terminalPanelHeight?: number;
  /** How wide the session sidebar is, in pixels. Absent means nobody has moved the divider. */
  readonly sidebarWidth?: number;
  /** Whether the session sidebar is on screen. On unless somebody puts it away. */
  readonly sidebarOpen: boolean;
  /**
   * Whether the host monitor runs beside the workspace. On unless somebody turns it off: hidden
   * means nothing is sampled either, not merely that nothing is drawn.
   */
  readonly monitorEnabled: boolean;
  /**
   * Whether a directory travels as one archive instead of file by file. Off unless somebody turns
   * it on: this changes what a transfer leaves on disk, so it is never decided for them.
   */
  readonly packedTransfer: boolean;
  /**
   * How much the interface is scaled, in percent of its own size — the same window, drawn larger.
   * One of `UI_SCALE_PERCENTS`; the Rust side reads anything else as the default.
   */
  readonly uiScale: number;
  /** How round the corners are: nothing, a little, or what this build has always drawn. */
  readonly cornerStyle: CornerStyle;
  /** The stack the interface draws its text in. Absent means the one this build ships with. */
  readonly uiFont?: string;
  /** The stack the terminal draws in, which is a choice of its own. */
  readonly terminalFont?: string;
  /** What a `custom` palette is drawn from. Absent means nobody has picked any yet. */
  readonly customColors?: CustomColors;
  /** The same for the dark theme, which has a set of its own. */
  readonly customColorsDark?: CustomColors;
  /** Absent means nobody has chosen a language, so the system's applies. */
  readonly locale?: string;
}
