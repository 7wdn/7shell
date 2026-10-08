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

/** What the home page is for, from `sevenshell_types::HomeLayout`. */
export type HomeLayout = "intro" | "workspace";

/**
 * The page a fresh install opens on: the introduction.
 *
 * A fresh install is a window with nothing connected in it, which is exactly who that page is written
 * for; somebody with hosts in the library is one setting away from the working page.
 */
export const DEFAULT_HOME_LAYOUT: HomeLayout = "intro";

/**
 * How the chrome around the panels is drawn, from `sevenshell_types::AppLayout`.
 *
 * Two shapes rather than a set of switches: they are two answers to "how much of the window should
 * be the thing I am looking at", and every difference between them falls out of that answer.
 */
export type AppLayout = "separated" | "classic";

/** The shape a fresh install gets: panels floating with the window showing between them. */
export const DEFAULT_APP_LAYOUT: AppLayout = "separated";

/**
 * What the tab bar does when the tabs no longer fit in one row, from `sevenshell_types::TabLayout`.
 *
 * One row that scrolls, or rows that wrap: the two answer whether the bar may take height from the
 * panel under it, which is why the choice is not a size like the panel height is.
 */
export type TabLayout = "scroll" | "wrap";

/** One row that scrolls, which is what this build has always drawn. */
export const DEFAULT_TAB_LAYOUT: TabLayout = "scroll";

/** Which edge of the window the tab bar is drawn against, from `sevenshell_types::TabPlacement`. */
export type TabPlacement = "top" | "bottom" | "left" | "right";

/** Across the top, which is where this build has always drawn it. */
export const DEFAULT_TAB_PLACEMENT: TabPlacement = "top";

/** What the tabs are grouped by, from `sevenshell_types::TabGroupMode`. */
export type TabGroupMode = "none" | "session";

/** One run of tabs, which is what this build has always drawn. */
export const DEFAULT_TAB_GROUP_MODE: TabGroupMode = "none";

/** The order the tabs are drawn in, from `sevenshell_types::TabSortMode`. */
export type TabSortMode = "manual" | "created-asc" | "title-asc";

/** The order they were opened in, which is also the one that can be dragged. */
export const DEFAULT_TAB_SORT_MODE: TabSortMode = "manual";

/** How wide a horizontal tab may be, from `sevenshell_types::TabMaxWidth`. */
export type TabMaxWidth = "unlimited" | "compact" | "medium" | "standard" | "wide";

/**
 * The pixels each step of the tab width stands for, or null where the name sets the width.
 *
 * The steps are the setting and the pixels are what drawing them means, which is why the numbers are
 * here rather than on the wire: somebody choosing "medium" is choosing a look, and a rebuild that
 * decided medium is a different number of pixels would be a change to the drawing rather than to the
 * choice. `wide` is what this build has always drawn.
 */
export const TAB_MAX_WIDTH_PX: Record<TabMaxWidth, number | null> = {
  unlimited: null,
  compact: 160,
  medium: 200,
  standard: 240,
  wide: 320,
};

/**
 * No limit at all, which is the first step the list offers and what a fresh install gets.
 */
export const DEFAULT_TAB_MAX_WIDTH: TabMaxWidth = "unlimited";

/**
 * How the picture behind the window is painted when it does not match the window's shape.
 *
 * From `sevenshell_types::BackgroundDisplayMode`: the five the desktop has taught everybody.
 */
export type BackgroundDisplayMode = "fill" | "fit" | "stretch" | "center" | "tile";

/** The modes, in the order the list offers them. */
export const BACKGROUND_DISPLAY_MODES: readonly BackgroundDisplayMode[] = ["fill", "fit", "stretch", "center", "tile"];

/** The most the window's surfaces may let through, in percent, and the most blur this build paints. */
export const BACKGROUND_TRANSPARENCY_MAX = 95;
export const BACKGROUND_BLUR_MAX = 20;

/**
 * Where the two picture sliders start: a seventh of the way to the picture, and four pixels of blur.
 *
 * Enough that the picture is clearly there, and little enough that the work is still what the window is
 * for; the blur takes the edges off a photograph so that text over it stays the first thing read.
 */
export const DEFAULT_BACKGROUND_TRANSPARENCY = 15;
export const DEFAULT_BACKGROUND_BLUR = 4;

/**
 * The sizes the terminal may be drawn at, in pixels, and the one a fresh install has.
 *
 * From `sevenshell_types`: a closed range with its ends on the slider, because a terminal is measured
 * in columns and lines and a size nobody offers is one nobody could get back to.
 */
export const TERMINAL_FONT_SIZE_MIN = 10;
export const TERMINAL_FONT_SIZE_MAX = 24;
export const DEFAULT_TERMINAL_FONT_SIZE = 13;

/** The picture drawn behind the whole window, from `sevenshell_types::BackgroundImage`. */
export interface BackgroundImage {
  /** What the picture is called, as the person who chose it knows it. No picture when absent. */
  readonly fileName?: string;
  readonly displayMode: BackgroundDisplayMode;
  /** How much the window's own surfaces let it through, in percent. */
  readonly transparency: number;
  readonly blur: number;
}

/** The picture a fresh install has: none, with the sliders saying where they would start. */
export const DEFAULT_BACKGROUND_IMAGE: BackgroundImage = {
  displayMode: "fill",
  transparency: DEFAULT_BACKGROUND_TRANSPARENCY,
  blur: DEFAULT_BACKGROUND_BLUR,
};

/**
 * Whether there is a picture at all.
 *
 * An empty name is no picture — the same rule as in Rust, where an optional field and a name of
 * spaces both mean the same thing. Written once here so that the interface has one answer rather than
 * three truthiness checks that could drift apart.
 */
export function backgroundIsSet(image: BackgroundImage): boolean {
  return (image.fileName ?? "").trim().length > 0;
}

/**
 * Whether the shell keeps an icon in the system tray.
 *
 * On unless somebody says otherwise: a window that puts itself away when it is closed needs somewhere
 * to be picked up again, and the tray is that somewhere.
 */
export const DEFAULT_SHOW_TRAY_ICON = true;

/**
 * Whether closing the window ends the application rather than putting it away.
 *
 * Off unless somebody turns it on, which is a *change* from what this build did before the choice
 * existed — closing used to end it, because there was no tray to put it away to.
 */
export const DEFAULT_QUIT_ON_CLOSE = false;

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
   * Whether the line under a terminal offers what the application knows while somebody types. On
   * unless somebody turns it off: the list appears beside the line and never writes into it, so it
   * does not type for anybody — while a build that had it off would be a feature nobody finds.
   */
  readonly commandCompletion: boolean;
  /**
   * The keys somebody has changed, by the action they belong to. An action that is not in here keeps
   * the default the build ships — which is what makes a build that adds one nothing to migrate for.
   * The value is a binding in the application's own spelling (`Mod+V`), the same string on every
   * platform: which physical key that means is decided where the key is listened for.
   */
  readonly keybindings?: Readonly<Record<string, string>>;
  /**
   * How much the interface is scaled, in percent of its own size — the same window, drawn larger.
   * One of `UI_SCALE_PERCENTS`; the Rust side reads anything else as the default.
   */
  readonly uiScale: number;
  /** How round the corners are: nothing, a little, or what this build has always drawn. */
  readonly cornerStyle: CornerStyle;
  /** Which of the two home pages is drawn. */
  readonly homeLayout: HomeLayout;
  /** How the chrome around the panels is drawn. */
  readonly appLayout: AppLayout;
  /** What the tab bar does when the tabs no longer fit in one row. */
  readonly tabLayout: TabLayout;
  /** Which edge the tab bar is drawn against. */
  readonly tabPlacement: TabPlacement;
  /** What the tabs are grouped by when they are drawn. */
  readonly tabGroupMode: TabGroupMode;
  /** The order the tabs are drawn in. */
  readonly tabSortMode: TabSortMode;
  /** How wide a horizontal tab may be drawn. */
  readonly tabMaxWidth: TabMaxWidth;
  /** The picture drawn behind the whole window, if any. */
  readonly backgroundImage: BackgroundImage;
  /** Whether the shell keeps an icon in the system tray. */
  readonly showTrayIcon: boolean;
  /** Whether closing the window ends the application rather than putting it away. */
  readonly quitOnClose: boolean;
  /** The stack the interface draws its text in. Absent means the one this build ships with. */
  readonly uiFont?: string;
  /** How large the terminal draws its text, in pixels. */
  readonly terminalFontSize: number;
  /** The stack the terminal draws in, which is a choice of its own. */
  readonly terminalFont?: string;
  /** What a `custom` palette is drawn from. Absent means nobody has picked any yet. */
  readonly customColors?: CustomColors;
  /** The same for the dark theme, which has a set of its own. */
  readonly customColorsDark?: CustomColors;
  /** Absent means nobody has chosen a language, so the system's applies. */
  readonly locale?: string;
}
