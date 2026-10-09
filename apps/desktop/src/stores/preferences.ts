/**
 * Appearance and language.
 *
 * Two rules shape this store:
 *
 * 1. **Nothing is chosen until someone chooses it.** "Follow the system" is a real mode, not a
 *    placeholder, so a fresh install follows the operating system's theme and language and needs
 *    no first-run dialog. The settings file only records a language once a person picks one.
 * 2. **The file is not the source of truth for the current session.** The store applies changes
 *    immediately and writes afterwards; a failed write is surfaced, not retried forever, because
 *    the user's choice still holds for this run.
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { DEFAULT_LOCALE, type SupportedLocale, i18n, isSupportedLocale, resolveLocale } from "@/i18n";
import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { deriveCustomColors } from "@/lib/settings/customColors";
import type { ShortcutActionId } from "@/lib/shortcuts/registry";
import {
  BACKGROUND_BLUR_MAX,
  TERMINAL_FONT_SIZE_MIN,
  TERMINAL_FONT_SIZE_MAX,
  BACKGROUND_TRANSPARENCY_MAX,
  DEFAULT_APP_LAYOUT,
  DEFAULT_BACKGROUND_IMAGE,
  DEFAULT_CORNER_STYLE,
  DEFAULT_QUIT_ON_CLOSE,
  DEFAULT_SHOW_TRAY_ICON,
  DEFAULT_TAB_GROUP_MODE,
  DEFAULT_TAB_LAYOUT,
  DEFAULT_TAB_MAX_WIDTH,
  DEFAULT_TAB_PLACEMENT,
  DEFAULT_TAB_SORT_MODE,
  DEFAULT_HOME_LAYOUT,
  DEFAULT_CUSTOM_COLORS,
  DEFAULT_CUSTOM_COLORS_DARK,
  DEFAULT_TERMINAL_FONT,
  DEFAULT_TERMINAL_FONT_SIZE,
  DEFAULT_UI_FONT,
  DEFAULT_UI_SCALE,
  type AppLayout,
  type AppSettings,
  type BackgroundDisplayMode,
  type BackgroundImage,
  type TabGroupMode,
  type TabLayout,
  type TabMaxWidth,
  type TabPlacement,
  type TabSortMode,
  type CornerStyle,
  type HomeLayout,
  type CustomColors,
  type FilesLayout,
  type ResolvedTheme,
  type SessionLayout,
  type ThemeMode,
  type ThemePalette,
} from "@/types/app";

const DARK_QUERY = "(prefers-color-scheme: dark)";

/** One field of the settings file, named the way the file names it. */
export type PreferenceField = keyof AppSettings;

/**
 * What each field is before anybody has chosen anything.
 *
 * The same answers `AppSettings::default()` gives on the far side, written once here so that
 * "restore the defaults" and a fresh install cannot drift apart. `locale` is deliberately absent:
 * its default is not a value but the absence of one, which is what leaves the system's language
 * applying again.
 */
const FRESH: Omit<AppSettings, "locale"> = {
  themeMode: "system",
  themePalette: "default",
  uiScale: DEFAULT_UI_SCALE,
  cornerStyle: DEFAULT_CORNER_STYLE,
  homeLayout: DEFAULT_HOME_LAYOUT,
  appLayout: DEFAULT_APP_LAYOUT,
  tabLayout: DEFAULT_TAB_LAYOUT,
  tabPlacement: DEFAULT_TAB_PLACEMENT,
  tabGroupMode: DEFAULT_TAB_GROUP_MODE,
  tabSortMode: DEFAULT_TAB_SORT_MODE,
  tabMaxWidth: DEFAULT_TAB_MAX_WIDTH,
  showTrayIcon: DEFAULT_SHOW_TRAY_ICON,
  quitOnClose: DEFAULT_QUIT_ON_CLOSE,
  backgroundImage: DEFAULT_BACKGROUND_IMAGE,
  uiFont: undefined,
  terminalFont: undefined,
  terminalFontSize: DEFAULT_TERMINAL_FONT_SIZE,
  customColors: DEFAULT_CUSTOM_COLORS,
  customColorsDark: DEFAULT_CUSTOM_COLORS_DARK,
  terminalTheme: "system",
  filesLayout: "tree",
  sessionLayout: "split",
  terminalPanelHeight: undefined,
  sidebarWidth: undefined,
  sidebarOpen: true,
  monitorEnabled: true,
  packedTransfer: false,
  commandCompletion: true,
  keybindings: {},
  syncEndpoint: "",
  syncUsername: "",
  syncRemotePath: "",
  syncIncludeSecrets: false,
};

/**
 * How wide the session sidebar starts out, until somebody moves the divider beside it.
 *
 * Wide enough for the panel's own toolbar — the import/export control and five buttons — without
 * the label being cut off, which is the point at which the panel stops saying what it is.
 */
export const DEFAULT_SIDEBAR_WIDTH = 280;

/**
 * How tall the file panel under a terminal starts out, until somebody moves the divider.
 *
 * About a dozen rows of a directory listing: enough to see where you are and pick a file out of it,
 * small enough that the terminal above it is still the larger half.
 */
export const DEFAULT_TERMINAL_PANEL_HEIGHT = 280;

export const usePreferencesStore = defineStore("preferences", () => {
  const themeMode = ref<ThemeMode>("system");
  /**
   * What colour the interface is made of.
   *
   * Orthogonal to the theme above, and chosen as its own field: the theme says whether the lights
   * are on, the palette says what colour they are. Every palette is drawn in both themes, so the
   * two never have to be reconciled.
   */
  const themePalette = ref<ThemePalette>("default");
  /**
   * A palette that is being looked at rather than chosen: the one under the pointer in the list.
   *
   * Kept apart from the choice because it is not one — it is written nowhere, it survives no
   * restart, and letting go of the list puts the chosen one back. A colour is the one setting whose
   * whole content is what it looks like, so it is the one setting worth showing before it is taken.
   */
  const previewedPalette = ref<ThemePalette | null>(null);
  /**
   * The five colours a custom palette is drawn from, in each theme.
   *
   * Two sets rather than one, because the two themes are chosen apart: a set made for a white
   * window is unreadable on a black one, so "custom" is really two palettes that happen to share a
   * name. What the other twelve palettes do not need — each of them is written for both.
   */
  const customColors = ref<CustomColors>({ ...DEFAULT_CUSTOM_COLORS });
  const customColorsDark = ref<CustomColors>({ ...DEFAULT_CUSTOM_COLORS_DARK });
  /**
   * How much the interface is scaled, in percent of the size it is drawn at.
   *
   * The window keeps its size — this is the interface inside it growing, which is what a display
   * dense enough to make 12px text unreadable needs and what the operating system's own scaling
   * cannot do for a window that is already open.
   */
  const uiScale = ref<number>(DEFAULT_UI_SCALE);
  /**
   * How round the corners are.
   *
   * It reaches the window the way the palette does — an attribute on the document — because the
   * roundness is a handful of numbers the stylesheet declares and this only says which set: a
   * component that rounded its own corners would be the one that ignored the choice.
   */
  const cornerStyle = ref<CornerStyle>(DEFAULT_CORNER_STYLE);
  /**
   * The stacks the interface and the terminal draw in, or null while nobody has chosen one.
   *
   * Null rather than the default's own stack, because "not chosen" and "chosen the default" are two
   * answers to two different questions: the first is what a reading of the settings file has to
   * report, and the second is a choice somebody made and can make again.
   */
  const uiFont = ref<string | null>(null);
  const terminalFont = ref<string | null>(null);
  /**
   * The family the pointer is on in one of the two lists, and which of the two it was.
   *
   * One ref for both because a preview can only ever be in one list at a time, and the field is
   * carried with the value so that hovering the terminal's list does not dress the interface.
   */
  const previewedFont = ref<{ field: "uiFont" | "terminalFont"; value: string } | null>(null);
  /** Which palette the terminal draws in; `system` means "follow the interface theme". */
  const terminalTheme = ref<ThemeMode>("system");
  /** Two panes or one, in the file browser. */
  const filesLayout = ref<FilesLayout>("tree");
  /**
   * How a session opens: a terminal on its own, or a terminal with its file panel under it.
   *
   * The default is the split, so the files for a host are there from the first moment without
   * anybody having to ask for them. It is a default rather than a rule: the panel can be closed
   * per tab, on the strip under the terminal.
   */
  const sessionLayout = ref<SessionLayout>("split");
  /**
   * How tall the file panel under a terminal is, in pixels; null until somebody moves the divider.
   *
   * A pixel height rather than a fraction of the window: the panel is a fixed-purpose thing — a
   * list of files, wanting a predictable number of rows — and the terminal above it is the half
   * that can absorb whatever the window does.
   */
  const terminalPanelHeight = ref<number | null>(null);
  /**
   * How wide the session sidebar is, in pixels; null until somebody moves the divider beside it.
   *
   * A pixel width, like the file panel's height above and for the same reason: the sidebar is a
   * fixed-purpose thing — a list of names — and the workspace beside it is the half that can
   * absorb whatever the window does.
   */
  const sidebarWidth = ref<number | null>(null);
  /**
   * Whether the sidebar is drawn at all.
   *
   * On unless somebody puts it away, and remembered: somebody who works with the sidebar collapsed
   * is somebody who wants it collapsed the next time they open the application.
   */
  const sidebarOpen = ref(true);
  /**
   * Which of the two home pages is drawn.
   *
   * Two pages rather than one page with parts that can be turned off: somebody who has never
   * connected anything wants to know what this is, and somebody who has wants their own hosts. The
   * pieces belong together — the counters and the host names are the same answer, not two.
   */
  const homeLayout = ref<HomeLayout>(DEFAULT_HOME_LAYOUT);
  /**
   * How the chrome around the panels is drawn.
   *
   * Not a preference about a colour or a size but about how much of the window is the thing being
   * looked at: tabs joined into one bar and tighter headers leave a row or two to the panel. The
   * choice reaches the stylesheet as an attribute rather than as classes, because what it changes is
   * spread over the strip, the headers and the panel frames.
   */
  const appLayout = ref<AppLayout>(DEFAULT_APP_LAYOUT);
  /**
   * What the tab bar does when the tabs no longer fit in one row.
   *
   * A chrome question like the one above, and answered by the strip itself rather than by the
   * stylesheet: wrapping changes how the bar measures, not only how it looks, and the panel under it
   * gives up the height. The setting is read where the tabs are drawn, which is the only place the
   * answer means anything.
   */
  const tabLayout = ref<TabLayout>(DEFAULT_TAB_LAYOUT);
  /** Which edge of the window the tab bar is drawn against. */
  const tabPlacement = ref<TabPlacement>(DEFAULT_TAB_PLACEMENT);
  /** What the tabs are grouped by when they are drawn. */
  const tabGroupMode = ref<TabGroupMode>(DEFAULT_TAB_GROUP_MODE);
  /** The order the tabs are drawn in. Sorting moves the drawing, never the tabs themselves. */
  const tabSortMode = ref<TabSortMode>(DEFAULT_TAB_SORT_MODE);
  /** How wide a horizontal tab may be drawn, as a step rather than as a number of pixels. */
  const tabMaxWidth = ref<TabMaxWidth>(DEFAULT_TAB_MAX_WIDTH);
  /**
   * Whether the shell keeps an icon in the system tray, and whether closing the window ends it.
   *
   * The two the shell itself obeys rather than the interface, which is why nothing here draws them:
   * they are written to the settings file, and the shell reads them from there. The file is the whole
   * of the handover — the interface never tells the shell what to do with its own window.
   */
  const showTrayIcon = ref(DEFAULT_SHOW_TRAY_ICON);
  const quitOnClose = ref(DEFAULT_QUIT_ON_CLOSE);
  /**
   * The picture behind the window.
   *
   * One value rather than four fields, because the four only mean anything together: a name, a mode and
   * two numbers. Nothing here *reads* the picture — that is the wallpaper's own business — and the rule
   * is the one every choice on this page follows: it takes effect in the window at once and is written
   * when somebody says so.
   */
  const backgroundImage = ref<BackgroundImage>({ ...DEFAULT_BACKGROUND_IMAGE });
  /**
   * Whether the host monitor is drawn beside the workspace.
   *
   * On unless somebody turns it off, which is not how the fields above behave: those describe a
   * choice the interface would otherwise guess at, while this one is a feature being there or not.
   * The panel is mounted only while this is true, and mounting it is what starts the sampling —
   * so "hidden" means the commands on the far side stop as well as the drawing.
   */
  const monitorEnabled = ref(true);
  /**
   * Whether a remote directory is moved as one archive rather than file by file.
   *
   * Off unless somebody says otherwise, which is the opposite of the monitor above and for the same
   * kind of reason: that one is a panel appearing, this one changes what a transfer leaves on the
   * disk. Somebody who right-clicks a folder and finds a `.tar.gz` where they expected the files
   * would be right to be annoyed, so it is never decided for them.
   */
  const packedTransfer = ref(false);
  /**
   * Whether the command line completes. On by default, and the only switch in the settings that turns
   * something in the interface *on*: it draws a list beside the line and waits to be taken, so somebody
   * who never presses Tab never sees it do anything.
   */
  const commandCompletion = ref(true);
  /**
   * The keys somebody has changed, by action. Empty means every action is the one the build ships,
   * which is what a fresh install has and what dropping the whole map means.
   */
  const keybindings = ref<Record<string, string>>({});
  const systemPrefersDark = ref(true);
  const locale = ref<SupportedLocale>(DEFAULT_LOCALE);
  /** True once a person has chosen a language, so the system's stops applying. */
  const localeChosen = ref(false);
  /**
   * Where a backup is sent: three fields and a switch, none of them a secret.
   *
   * The address, the user name and the path live in the settings file because that is where this
   * installation's configuration lives — the *password* is the one part that has to be behind the
   * vault's lock, and it is not in this store at all. An empty address is nobody having configured
   * a server, which is the same state as a fresh install.
   */
  const syncEndpoint = ref("");
  const syncUsername = ref("");
  const syncRemotePath = ref("");
  const syncIncludeSecrets = ref(false);
  const problem = ref<ShellError | null>(null);

  const resolvedTheme = computed<ResolvedTheme>(() => {
    if (themeMode.value === "system") {
      return systemPrefersDark.value ? "dark" : "light";
    }
    return themeMode.value;
  });

  /**
   * Which palette the terminal uses.
   *
   * `system` resolves to the interface's own answer, never to the operating system's: a terminal
   * that followed the system while the window around it did not would be the one thing on screen
   * that disagrees. Always dark, or always light, are both real choices and both kept.
   */
  const resolvedTerminalTheme = computed<ResolvedTheme>(() => {
    if (terminalTheme.value === "system") {
      return resolvedTheme.value;
    }
    return terminalTheme.value;
  });

  /**
   * What colour the interface is drawn in: the option under the pointer, or the chosen one.
   *
   * One answer rather than two, so that nothing else in the store has to know a preview is on —
   * `apply` writes this, and the choice itself is left exactly where it was.
   */
  const wornPalette = computed<ThemePalette>(() => previewedPalette.value ?? themePalette.value);

  /** What the interface draws in: the family under the pointer, or the one chosen. */
  const wornUiFont = computed(() =>
    previewedFont.value?.field === "uiFont" ? previewedFont.value.value : (uiFont.value ?? DEFAULT_UI_FONT),
  );

  /**
   * How large the terminal draws its text.
   *
   * Its own setting rather than a share of the interface scale: the two are read for different
   * reasons, and somebody who wants their shell at sixteen pixels has not said anything about the size
   * of the window's own text.
   */
  const terminalFontSize = ref(DEFAULT_TERMINAL_FONT_SIZE);

  /** The same for the terminal, which is a choice of its own. */
  const wornTerminalFont = computed(() =>
    previewedFont.value?.field === "terminalFont"
      ? previewedFont.value.value
      : (terminalFont.value ?? DEFAULT_TERMINAL_FONT),
  );

  /** What the divider is drawn at: the height somebody chose, or the starting one. */
  const resolvedPanelHeight = computed(() => terminalPanelHeight.value ?? DEFAULT_TERMINAL_PANEL_HEIGHT);

  /** The same answer for the sidebar's width. */
  const resolvedSidebarWidth = computed(() => sidebarWidth.value ?? DEFAULT_SIDEBAR_WIDTH);

  /**
   * What the shell was last asked to draw the interface at, or null when it never was.
   *
   * Kept so that applying is not a call across the boundary per theme change: `apply` runs whenever
   * the system's own preference moves, and the scale has nothing to do with that.
   */
  let appliedScale: number | null = null;

  /**
   * Hands the scale to the shell, which draws the whole webview at it.
   *
   * A failure here is the one thing a person can see and cannot explain — the interface simply
   * stays small after being told to grow — so it is reported the same way a failed write is.
   */
  function applyScale(): void {
    const scale = uiScale.value / 100;
    if (scale === appliedScale) {
      return;
    }
    appliedScale = scale;
    void backend.window.setZoom(scale).catch((error: unknown) => {
      appliedScale = null;
      problem.value = asShellError(error, "window_set_zoom");
    });
  }

  /** The tokens the last custom palette wrote inline, so the next one can take them back. */
  let writtenCustomTokens: string[] = [];

  /**
   * Writes a hand-picked palette as tokens on the document, or takes the last one back.
   *
   * The twelve named palettes are stylesheet blocks selected by an attribute; this one cannot be,
   * because its colours are whatever somebody picked a moment ago. Inline custom properties are the
   * same mechanism one step further along — they win over the stylesheet, and removing them is how
   * the named palettes become visible again.
   *
   * Both sets are written, not only the one on screen: the pane's own palettes are named by theme
   * (`--terminal-dark-*`, `--terminal-light-*`) and the terminal draws in whichever it was told to,
   * which is not always the theme of the window around it.
   */
  function applyCustomColors(): void {
    const root = document.documentElement;
    for (const name of writtenCustomTokens) {
      root.style.removeProperty(name);
    }
    writtenCustomTokens = [];

    if (wornPalette.value !== "custom") {
      return;
    }

    // Derived one per theme rather than one per the theme on screen: each call names its own half
    // of the pane's tokens, and the window's tokens only come from the half that is in force.
    const light = deriveCustomColors(customColors.value, "light");
    const dark = deriveCustomColors(customColorsDark.value, "dark");
    const inForce = resolvedTheme.value === "dark" ? dark : light;

    for (const [name, value] of Object.entries(inForce).filter(([name]) => !name.startsWith("--terminal-"))) {
      root.style.setProperty(name, value);
      writtenCustomTokens.push(name);
    }
    for (const [name, value] of Object.entries({ ...light, ...dark })) {
      if (!name.startsWith("--terminal-")) {
        continue;
      }
      root.style.setProperty(name, value);
      writtenCustomTokens.push(name);
    }
  }

  function apply(): void {
    document.documentElement.classList.toggle("dark", resolvedTheme.value === "dark");
    document.documentElement.dataset.theme = resolvedTheme.value;
    document.documentElement.dataset.palette = wornPalette.value;
    document.documentElement.dataset.cornerStyle = cornerStyle.value;
    document.documentElement.dataset.appLayout = appLayout.value;
    document.documentElement.lang = locale.value;
    i18n.global.locale.value = locale.value;
    applyCustomColors();
    applyFonts();
    applyScale();
  }

  /** Follows the operating system while the theme mode is "system". */
  function followSystemTheme(): () => void {
    if (typeof window.matchMedia !== "function") {
      return () => {};
    }
    const query = window.matchMedia(DARK_QUERY);
    systemPrefersDark.value = query.matches;

    const onChange = (event: MediaQueryListEvent): void => {
      systemPrefersDark.value = event.matches;
      apply();
    };
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }

  async function load(): Promise<void> {
    problem.value = null;
    // Whatever was being shown belongs to the settings that were on screen before this read.
    previewedPalette.value = null;
    previewedFont.value = null;
    try {
      const settings = await backend.settings.get();
      themeMode.value = settings.themeMode;
      themePalette.value = settings.themePalette;
      uiScale.value = settings.uiScale;
      cornerStyle.value = settings.cornerStyle;
      homeLayout.value = settings.homeLayout;
      appLayout.value = settings.appLayout;
      tabLayout.value = settings.tabLayout;
      tabPlacement.value = settings.tabPlacement;
      tabGroupMode.value = settings.tabGroupMode;
      tabSortMode.value = settings.tabSortMode;
      tabMaxWidth.value = settings.tabMaxWidth;
      showTrayIcon.value = settings.showTrayIcon;
      quitOnClose.value = settings.quitOnClose;
      backgroundImage.value = settings.backgroundImage;
      uiFont.value = settings.uiFont ?? null;
      terminalFont.value = settings.terminalFont ?? null;
      terminalFontSize.value = settings.terminalFontSize;
      customColors.value = settings.customColors ?? { ...DEFAULT_CUSTOM_COLORS };
      customColorsDark.value = settings.customColorsDark ?? { ...DEFAULT_CUSTOM_COLORS_DARK };
      terminalTheme.value = settings.terminalTheme;
      filesLayout.value = settings.filesLayout;
      sessionLayout.value = settings.sessionLayout;
      terminalPanelHeight.value = settings.terminalPanelHeight ?? null;
      sidebarWidth.value = settings.sidebarWidth ?? null;
      sidebarOpen.value = settings.sidebarOpen;
      monitorEnabled.value = settings.monitorEnabled;
      packedTransfer.value = settings.packedTransfer;
      commandCompletion.value = settings.commandCompletion;
      keybindings.value = { ...settings.keybindings };
      syncEndpoint.value = settings.syncEndpoint ?? "";
      syncUsername.value = settings.syncUsername ?? "";
      syncRemotePath.value = settings.syncRemotePath ?? "";
      syncIncludeSecrets.value = settings.syncIncludeSecrets;
      if (settings.locale && isSupportedLocale(settings.locale)) {
        locale.value = settings.locale;
        localeChosen.value = true;
      } else {
        locale.value = resolveLocale(typeof navigator === "undefined" ? undefined : navigator.language);
      }
    } catch (error) {
      // A settings file that cannot be read must not stop the interface from opening: the
      // defaults are used and the reason is surfaced in the settings page.
      problem.value = asShellError(error, "settings_get");
      locale.value = resolveLocale(typeof navigator === "undefined" ? undefined : navigator.language);
    }
    apply();
  }

  /**
   * Whether the window is showing something the settings file has not been told about yet.
   *
   * The two are not the same thing and this is the difference: a choice is *applied* the moment it is
   * made — a colour nobody can see before choosing it is not a choice — and it is *written* when
   * somebody says so, or when the page is left. That is what the button under the page is about, and
   * it is also why dragging the native colour picker no longer writes the file on every mouse move.
   */
  const dirty = ref(false);

  /** Writes down what is on screen, and says whether it worked. */
  async function commit(): Promise<boolean> {
    await persist();
    if (problem.value === null) {
      dirty.value = false;
      return true;
    }
    return false;
  }

  /**
   * Throws away what has been chosen since the last write.
   *
   * Reading the file again rather than keeping a copy of it: there is one answer to "what has been
   * written down", and it is the file.
   */
  async function discard(): Promise<void> {
    await load();
    // What is on screen is the file again, so there is nothing left to write.
    dirty.value = false;
  }

  async function persist(): Promise<void> {
    problem.value = null;
    try {
      await backend.settings.set({
        themeMode: themeMode.value,
        themePalette: themePalette.value,
        uiScale: uiScale.value,
        cornerStyle: cornerStyle.value,
        homeLayout: homeLayout.value,
        appLayout: appLayout.value,
        tabLayout: tabLayout.value,
        tabPlacement: tabPlacement.value,
        tabGroupMode: tabGroupMode.value,
        tabSortMode: tabSortMode.value,
        tabMaxWidth: tabMaxWidth.value,
        showTrayIcon: showTrayIcon.value,
        quitOnClose: quitOnClose.value,
        backgroundImage: backgroundImage.value,
        uiFont: uiFont.value ?? undefined,
        terminalFont: terminalFont.value ?? undefined,
        terminalFontSize: terminalFontSize.value,
        customColors: customColors.value,
        customColorsDark: customColorsDark.value,
        terminalTheme: terminalTheme.value,
        filesLayout: filesLayout.value,
        sessionLayout: sessionLayout.value,
        terminalPanelHeight: terminalPanelHeight.value ?? undefined,
        sidebarWidth: sidebarWidth.value ?? undefined,
        sidebarOpen: sidebarOpen.value,
        monitorEnabled: monitorEnabled.value,
        packedTransfer: packedTransfer.value,
        commandCompletion: commandCompletion.value,
        keybindings: { ...keybindings.value },
        locale: localeChosen.value ? locale.value : undefined,
        // A field nobody filled in is written as absent rather than as an empty string, which is
        // what "no server configured" means in the file and what a half-filled form must not stop
        // meaning.
        syncEndpoint: syncEndpoint.value.trim() || undefined,
        syncUsername: syncUsername.value.trim() || undefined,
        syncRemotePath: syncRemotePath.value.trim() || undefined,
        syncIncludeSecrets: syncIncludeSecrets.value,
      });
    } catch (error) {
      problem.value = asShellError(error, "settings_set");
    }
  }

  async function setThemeMode(mode: ThemeMode): Promise<void> {
    themeMode.value = mode;
    apply();
    dirty.value = true;
  }

  /**
   * Changes what colour the interface is made of, and writes it down.
   *
   * Applied before it is written, like the theme above: the palette is a set of tokens the
   * stylesheet already holds, so choosing one shows it — waiting for a file to be written first
   * would be waiting for nothing a person can see.
   */
  async function setThemePalette(palette: ThemePalette): Promise<void> {
    themePalette.value = palette;
    // The preview has served its purpose: what is on screen from here is the choice, and leaving
    // the preview standing would mean the list and the window disagreed about which was in force.
    previewedPalette.value = null;
    apply();
    dirty.value = true;
  }

  /**
   * Shows a palette without choosing it: the option the pointer is on.
   *
   * Nothing is written and nothing is remembered. Hovering the one already chosen is not a preview
   * at all — there would be nothing to show — so it clears whatever was being shown instead.
   */
  function previewThemePalette(palette: ThemePalette): void {
    if (palette === wornPalette.value) {
      return;
    }
    previewedPalette.value = palette === themePalette.value ? null : palette;
    apply();
  }

  /** Puts down what was being shown, which is what a pointer leaving the list means. */
  function clearThemePalettePreview(): void {
    if (previewedPalette.value === null) {
      return;
    }
    previewedPalette.value = null;
    apply();
  }

  /**
   * Writes the interface's stack onto the document, which is where the interface reads it.
   *
   * One of the two fonts goes here and the other does not, and the difference is deliberate. The
   * interface is drawn from tokens, so its font is a token. The terminal is not: it is a canvas with
   * one font handed to it, and the token that would carry a terminal's font — `--font-mono` — is the
   * face this whole window uses for paths, fingerprints and codes. Writing the terminal's choice
   * there would dress every reading on every page in it, which is not what somebody choosing a
   * terminal font asked for. So the pane reads the store, and this writes one thing.
   */
  function applyFonts(): void {
    document.documentElement.style.setProperty("--font-sans", wornUiFont.value);
  }

  /**
   * Shows a family without choosing it: the row the pointer is on, or the one a key landed on.
   *
   * Nothing is written and nothing is remembered. A terminal that is already open follows, because
   * the family reaches the pane as a token rather than as a decision — the pane watches it.
   */
  function previewFont(field: "uiFont" | "terminalFont", value: string): void {
    if (previewedFont.value?.field === field && previewedFont.value.value === value) {
      return;
    }
    previewedFont.value = { field, value };
    apply();
  }

  /** Puts down what was being shown, which is what leaving the list means. */
  function clearFontPreview(): void {
    if (previewedFont.value === null) {
      return;
    }
    previewedFont.value = null;
    apply();
  }

  /** Chooses which of the two home pages is drawn, which is the page itself being redrawn. */
  function setHomeLayout(layout: HomeLayout): void {
    homeLayout.value = layout;
    dirty.value = true;
  }

  /** Chooses the chrome's shape, which the whole window is redrawn into at once. */
  async function setAppLayout(layout: AppLayout): Promise<void> {
    appLayout.value = layout;
    apply();
    dirty.value = true;
  }

  /** Chooses what the tab bar does when the tabs do not fit, which the bar redraws itself into. */
  function setTabLayout(layout: TabLayout): void {
    tabLayout.value = layout;
    dirty.value = true;
  }

  /**
   * The four the bar is drawn by: where it sits, how it groups, how it orders and how wide it draws.
   *
   * All four are the bar's own business rather than the window's, so they are one setter's worth of
   * the same thing: each takes effect the moment it is chosen, because the bar is on screen behind the
   * page that chose it.
   */
  function setTabPlacement(edge: TabPlacement): void {
    tabPlacement.value = edge;
    dirty.value = true;
  }

  function setTabGroupMode(mode: TabGroupMode): void {
    tabGroupMode.value = mode;
    dirty.value = true;
  }

  function setTabSortMode(mode: TabSortMode): void {
    tabSortMode.value = mode;
    dirty.value = true;
  }

  function setTabMaxWidth(width: TabMaxWidth): void {
    tabMaxWidth.value = width;
    dirty.value = true;
  }

  /** Names the picture the shell has just copied in, or clears the name when it is taken away. */
  function setBackgroundImageName(name: string | null): void {
    const trimmed = (name ?? "").trim();
    backgroundImage.value = { ...backgroundImage.value, fileName: trimmed.length > 0 ? trimmed : undefined };
    dirty.value = true;
  }

  /** How the picture is fitted into a window it does not match. */
  function setBackgroundDisplayMode(mode: BackgroundDisplayMode): void {
    backgroundImage.value = { ...backgroundImage.value, displayMode: mode };
    dirty.value = true;
  }

  /** How much of the picture the surfaces let through, in percent, kept inside its slider's ends. */
  function setBackgroundTransparency(percent: number): void {
    backgroundImage.value = {
      ...backgroundImage.value,
      transparency: Math.min(Math.max(Math.round(percent), 0), BACKGROUND_TRANSPARENCY_MAX),
    };
    dirty.value = true;
  }

  /** How far the picture is blurred, in pixels, kept inside its slider's ends. */
  function setBackgroundBlur(pixels: number): void {
    backgroundImage.value = {
      ...backgroundImage.value,
      blur: Math.min(Math.max(Math.round(pixels), 0), BACKGROUND_BLUR_MAX),
    };
    dirty.value = true;
  }

  /** The two the shell obeys: taking effect on the way out of this page, like every other write. */
  function setShowTrayIcon(shown: boolean): void {
    showTrayIcon.value = shown;
    dirty.value = true;
  }

  function setQuitOnClose(quit: boolean): void {
    quitOnClose.value = quit;
    dirty.value = true;
  }

  /** Chooses how round the corners are, which is felt at once — the whole window is redrawn. */
  async function setCornerStyle(style: CornerStyle): Promise<void> {
    cornerStyle.value = style;
    apply();
    dirty.value = true;
  }

  /** Chooses what the interface draws in, applied before it is written down. */
  async function setUiFont(stack: string): Promise<void> {
    uiFont.value = stack;
    // The preview has served its purpose: what is on screen from here is the choice.
    previewedFont.value = null;
    apply();
    dirty.value = true;
  }

  /** Chooses how large the terminal draws, kept inside the range its own slider offers. */
  function setTerminalFontSize(size: number): void {
    terminalFontSize.value = Math.min(Math.max(Math.round(size), TERMINAL_FONT_SIZE_MIN), TERMINAL_FONT_SIZE_MAX);
    dirty.value = true;
  }

  /** The same for the terminal, whose choice is its own. */
  async function setTerminalFont(stack: string): Promise<void> {
    terminalFont.value = stack;
    previewedFont.value = null;
    apply();
    dirty.value = true;
  }

  /**
   * Changes one colour of the hand-picked set, in the theme that is on screen.
   *
   * Which set is being edited follows the theme rather than being asked for: the pickers are drawn
   * beside the page they are changing, and a light window showing the dark set's colours would be a
   * form about something nobody can see.
   */
  async function setCustomColor(key: keyof CustomColors, colour: string): Promise<void> {
    const dark = resolvedTheme.value === "dark";
    const current = dark ? customColorsDark.value : customColors.value;
    const next = { ...current, [key]: colour };
    if (dark) {
      customColorsDark.value = next;
    } else {
      customColors.value = next;
    }
    apply();
    dirty.value = true;
  }

  /** Puts the set being edited back to what a fresh install starts from. */
  async function resetCustomColors(): Promise<void> {
    if (resolvedTheme.value === "dark") {
      customColorsDark.value = { ...DEFAULT_CUSTOM_COLORS_DARK };
    } else {
      customColors.value = { ...DEFAULT_CUSTOM_COLORS };
    }
    apply();
    dirty.value = true;
  }

  /**
   * Scales the interface, which is felt the moment it is chosen.
   *
   * It used to write before it applied, so that a window refusing the size would have the last word
   * over the write that had just succeeded. Nothing is written here any more — the page's Apply is
   * what writes — so the two cannot race at all.
   */
  function setUiScale(percent: number): void {
    uiScale.value = percent;
    apply();
    dirty.value = true;
  }

  async function setTerminalTheme(mode: ThemeMode): Promise<void> {
    terminalTheme.value = mode;
    dirty.value = true;
  }

  async function setFilesLayout(layout: FilesLayout): Promise<void> {
    filesLayout.value = layout;
    dirty.value = true;
  }

  async function setSessionLayout(layout: SessionLayout): Promise<void> {
    sessionLayout.value = layout;
    dirty.value = true;
  }

  /**
   * Shows or hides the host monitor.
   *
   * A setting rather than something to ask for on the panel itself, because the answer is the same
   * every time for the same person — and hiding it is what stops the sampling, so it is written
   * down rather than forgotten at the end of a run.
   */
  async function setMonitorEnabled(enabled: boolean): Promise<void> {
    monitorEnabled.value = enabled;
    dirty.value = true;
  }

  /**
   * Turns packed transfers on or off.
   *
   * Two ways in — the file panel's own menu and the settings — and one value behind them, so the
   * checkbox in the menu and the switch in the dialog cannot come to disagree.
   */
  async function setPackedTransfer(enabled: boolean): Promise<void> {
    packedTransfer.value = enabled;
    dirty.value = true;
  }

  /** Turns the command line's completion on or off. */
  function setCommandCompletion(enabled: boolean): void {
    commandCompletion.value = enabled;
    dirty.value = true;
  }

  /**
   * Where a backup is sent: the address, the user name and the path inside it.
   *
   * Written with the rest of the settings — this page's Apply — rather than saved by its own button.
   * The panel that uses them commits first, which is what makes "test this server" test the address
   * that is on screen rather than the one that was last written down.
   */
  function setSyncEndpoint(endpoint: string): void {
    syncEndpoint.value = endpoint;
    dirty.value = true;
  }

  function setSyncUsername(name: string): void {
    syncUsername.value = name;
    dirty.value = true;
  }

  function setSyncRemotePath(path: string): void {
    syncRemotePath.value = path;
    dirty.value = true;
  }

  /**
   * Whether an upload carries the stored passwords and keys.
   *
   * A preference rather than a promise: nothing is carried unless a passphrase is saved as well, and
   * the file says what it actually holds either way.
   */
  function setSyncIncludeSecrets(included: boolean): void {
    syncIncludeSecrets.value = included;
    dirty.value = true;
  }

  /**
   * Changes what one action answers to.
   *
   * Passing nothing puts the action back to the default, which is stored as *not being in the map at
   * all* rather than as a copy of a default that a later build might change. An empty string is a
   * decision of its own — a key somebody does not want to keep answering to — and stays written down.
   */
  function setKeybinding(id: ShortcutActionId, binding: string | null): void {
    const next = { ...keybindings.value };
    if (binding === null) {
      delete next[id];
    } else {
      next[id] = binding;
    }
    keybindings.value = next;
    dirty.value = true;
  }

  /**
   * Moves the divider.
   *
   * Applied at once and written down separately: a drag is dozens of moves, and every write is a
   * synced file. Nothing is lost by waiting — the height is a convenience, not a setting anyone
   * depends on being durable to the second.
   */
  function setPanelHeight(height: number): void {
    terminalPanelHeight.value = Math.round(height);
  }

  /**
   * Writes down the height the divider was left at.
   *
   * A drag is a gesture rather than a choice waiting to be applied — there is nothing on the page to
   * press afterwards — so this writes, and writing takes what else has been applied with it.
   */
  async function rememberPanelHeight(): Promise<void> {
    await commit();
  }

  /** Puts the divider back where it started, and forgets the height: nothing has been chosen. */
  async function resetPanelHeight(): Promise<void> {
    terminalPanelHeight.value = null;
    await commit();
  }

  /** Moves the sidebar's divider, applied at once and written down when the drag ends. */
  function setSidebarWidth(width: number): void {
    sidebarWidth.value = Math.round(width);
  }

  /** Writes down the width the sidebar's divider was left at, for the reason the height does. */
  async function rememberSidebarWidth(): Promise<void> {
    await commit();
  }

  /** Puts the divider back where it started, and forgets the width: nothing has been chosen. */
  async function resetSidebarWidth(): Promise<void> {
    sidebarWidth.value = null;
    await commit();
  }

  /**
   * Puts the sidebar away, or brings it back.
   *
   * Written down at once rather than at the end of a drag: it is one decision, and somebody who
   * closed the sidebar and quit meant it.
   */
  async function setSidebarOpen(open: boolean): Promise<void> {
    sidebarOpen.value = open;
    dirty.value = true;
  }

  async function setLocale(next: SupportedLocale): Promise<void> {
    localeChosen.value = true;
    locale.value = next;
    apply();
    dirty.value = true;
  }

  /**
   * Puts the named fields back to what a fresh install has, in one write.
   *
   * Several fields at once on purpose: "restore this category" is one decision, and a write per
   * field would be several files written for it — each of them able to fail on its own. The
   * language goes back to "not chosen" rather than to a concrete one, which is what lets the
   * system's language apply again.
   */
  async function reset(fields: readonly PreferenceField[]): Promise<void> {
    // A preview is not a field and is not restored: it is something the pointer was doing.
    previewedPalette.value = null;
    previewedFont.value = null;
    for (const field of fields) {
      switch (field) {
        case "themeMode":
          themeMode.value = FRESH.themeMode;
          break;
        case "themePalette":
          themePalette.value = FRESH.themePalette;
          break;
        case "uiScale":
          uiScale.value = FRESH.uiScale;
          break;
        case "cornerStyle":
          cornerStyle.value = FRESH.cornerStyle;
          break;
        case "homeLayout":
          homeLayout.value = FRESH.homeLayout;
          break;
        case "appLayout":
          appLayout.value = FRESH.appLayout;
          apply();
          break;
        case "tabLayout":
          tabLayout.value = FRESH.tabLayout;
          break;
        case "tabPlacement":
          tabPlacement.value = FRESH.tabPlacement;
          break;
        case "tabGroupMode":
          tabGroupMode.value = FRESH.tabGroupMode;
          break;
        case "tabSortMode":
          tabSortMode.value = FRESH.tabSortMode;
          break;
        case "tabMaxWidth":
          tabMaxWidth.value = FRESH.tabMaxWidth;
          break;
        case "showTrayIcon":
          showTrayIcon.value = FRESH.showTrayIcon;
          break;
        case "quitOnClose":
          quitOnClose.value = FRESH.quitOnClose;
          break;
        case "backgroundImage":
          backgroundImage.value = { ...FRESH.backgroundImage };
          break;
        case "uiFont":
          uiFont.value = null;
          break;
        case "terminalFont":
          terminalFont.value = null;
          break;
        case "terminalFontSize":
          terminalFontSize.value = DEFAULT_TERMINAL_FONT_SIZE;
          break;
        case "customColors":
          customColors.value = { ...DEFAULT_CUSTOM_COLORS };
          break;
        case "customColorsDark":
          customColorsDark.value = { ...DEFAULT_CUSTOM_COLORS_DARK };
          break;
        case "terminalTheme":
          terminalTheme.value = FRESH.terminalTheme;
          break;
        case "filesLayout":
          filesLayout.value = FRESH.filesLayout;
          break;
        case "sessionLayout":
          sessionLayout.value = FRESH.sessionLayout;
          break;
        case "terminalPanelHeight":
          terminalPanelHeight.value = null;
          break;
        case "sidebarWidth":
          sidebarWidth.value = null;
          break;
        case "sidebarOpen":
          sidebarOpen.value = FRESH.sidebarOpen;
          break;
        case "monitorEnabled":
          monitorEnabled.value = FRESH.monitorEnabled;
          break;
        case "packedTransfer":
          packedTransfer.value = FRESH.packedTransfer;
          break;
        case "commandCompletion":
          commandCompletion.value = FRESH.commandCompletion;
          break;
        case "keybindings":
          keybindings.value = {};
          break;
        case "locale":
          localeChosen.value = false;
          locale.value = resolveLocale(typeof navigator === "undefined" ? undefined : navigator.language);
          break;
      }
    }
    apply();
    dirty.value = true;
  }

  /** Applies the environment's preference and starts following the system. */
  function initialize(): () => void {
    const stopFollowing = followSystemTheme();
    apply();
    return stopFollowing;
  }

  return {
    themeMode,
    themePalette,
    wornPalette,
    uiScale,
    cornerStyle,
    homeLayout,
    appLayout,
    tabLayout,
    tabPlacement,
    tabGroupMode,
    tabSortMode,
    tabMaxWidth,
    showTrayIcon,
    quitOnClose,
    backgroundImage,
    uiFont,
    terminalFont,
    terminalFontSize,
    wornUiFont,
    wornTerminalFont,
    customColors,
    customColorsDark,
    terminalTheme,
    filesLayout,
    sessionLayout,
    terminalPanelHeight,
    sidebarWidth,
    sidebarOpen,
    monitorEnabled,
    packedTransfer,
    commandCompletion,
    keybindings,
    resolvedTheme,
    resolvedTerminalTheme,
    resolvedPanelHeight,
    resolvedSidebarWidth,
    locale,
    syncEndpoint,
    syncUsername,
    syncRemotePath,
    syncIncludeSecrets,
    problem,
    dirty,
    initialize,
    load,
    commit,
    discard,
    setThemeMode,
    setThemePalette,
    previewThemePalette,
    clearThemePalettePreview,
    setUiScale,
    setCornerStyle,
    setHomeLayout,
    setAppLayout,
    setTabLayout,
    setTabPlacement,
    setTabGroupMode,
    setTabSortMode,
    setTabMaxWidth,
    setShowTrayIcon,
    setQuitOnClose,
    setBackgroundImageName,
    setBackgroundDisplayMode,
    setBackgroundTransparency,
    setBackgroundBlur,
    setUiFont,
    setTerminalFont,
    setTerminalFontSize,
    previewFont,
    clearFontPreview,
    setCustomColor,
    resetCustomColors,
    setTerminalTheme,
    setFilesLayout,
    setSessionLayout,
    setMonitorEnabled,
    setPackedTransfer,
    setCommandCompletion,
    setSyncEndpoint,
    setSyncUsername,
    setSyncRemotePath,
    setSyncIncludeSecrets,
    setKeybinding,
    setPanelHeight,
    rememberPanelHeight,
    resetPanelHeight,
    setSidebarWidth,
    rememberSidebarWidth,
    resetSidebarWidth,
    setSidebarOpen,
    setLocale,
    reset,
  };
});
