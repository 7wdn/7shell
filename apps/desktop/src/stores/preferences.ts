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
import type { AppSettings, FilesLayout, ResolvedTheme, SessionLayout, ThemeMode } from "@/types/app";

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
  terminalTheme: "system",
  filesLayout: "tree",
  sessionLayout: "split",
  terminalPanelHeight: undefined,
  sidebarWidth: undefined,
  sidebarOpen: true,
  monitorEnabled: true,
  packedTransfer: false,
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
  const systemPrefersDark = ref(true);
  const locale = ref<SupportedLocale>(DEFAULT_LOCALE);
  /** True once a person has chosen a language, so the system's stops applying. */
  const localeChosen = ref(false);
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

  /** What the divider is drawn at: the height somebody chose, or the starting one. */
  const resolvedPanelHeight = computed(() => terminalPanelHeight.value ?? DEFAULT_TERMINAL_PANEL_HEIGHT);

  /** The same answer for the sidebar's width. */
  const resolvedSidebarWidth = computed(() => sidebarWidth.value ?? DEFAULT_SIDEBAR_WIDTH);

  function apply(): void {
    document.documentElement.classList.toggle("dark", resolvedTheme.value === "dark");
    document.documentElement.dataset.theme = resolvedTheme.value;
    document.documentElement.lang = locale.value;
    i18n.global.locale.value = locale.value;
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
    try {
      const settings = await backend.settings.get();
      themeMode.value = settings.themeMode;
      terminalTheme.value = settings.terminalTheme;
      filesLayout.value = settings.filesLayout;
      sessionLayout.value = settings.sessionLayout;
      terminalPanelHeight.value = settings.terminalPanelHeight ?? null;
      sidebarWidth.value = settings.sidebarWidth ?? null;
      sidebarOpen.value = settings.sidebarOpen;
      monitorEnabled.value = settings.monitorEnabled;
      packedTransfer.value = settings.packedTransfer;
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

  async function persist(): Promise<void> {
    problem.value = null;
    try {
      await backend.settings.set({
        themeMode: themeMode.value,
        terminalTheme: terminalTheme.value,
        filesLayout: filesLayout.value,
        sessionLayout: sessionLayout.value,
        terminalPanelHeight: terminalPanelHeight.value ?? undefined,
        sidebarWidth: sidebarWidth.value ?? undefined,
        sidebarOpen: sidebarOpen.value,
        monitorEnabled: monitorEnabled.value,
        packedTransfer: packedTransfer.value,
        locale: localeChosen.value ? locale.value : undefined,
      });
    } catch (error) {
      problem.value = asShellError(error, "settings_set");
    }
  }

  async function setThemeMode(mode: ThemeMode): Promise<void> {
    themeMode.value = mode;
    apply();
    await persist();
  }

  async function setTerminalTheme(mode: ThemeMode): Promise<void> {
    terminalTheme.value = mode;
    await persist();
  }

  async function setFilesLayout(layout: FilesLayout): Promise<void> {
    filesLayout.value = layout;
    await persist();
  }

  async function setSessionLayout(layout: SessionLayout): Promise<void> {
    sessionLayout.value = layout;
    await persist();
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
    await persist();
  }

  /**
   * Turns packed transfers on or off.
   *
   * Two ways in — the file panel's own menu and the settings — and one value behind them, so the
   * checkbox in the menu and the switch in the dialog cannot come to disagree.
   */
  async function setPackedTransfer(enabled: boolean): Promise<void> {
    packedTransfer.value = enabled;
    await persist();
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

  /** Writes down the height the divider was left at. */
  async function rememberPanelHeight(): Promise<void> {
    await persist();
  }

  /** Puts the divider back where it started, and forgets the height: nothing has been chosen. */
  async function resetPanelHeight(): Promise<void> {
    terminalPanelHeight.value = null;
    await persist();
  }

  /** Moves the sidebar's divider, applied at once and written down when the drag ends. */
  function setSidebarWidth(width: number): void {
    sidebarWidth.value = Math.round(width);
  }

  /** Writes down the width the sidebar's divider was left at. */
  async function rememberSidebarWidth(): Promise<void> {
    await persist();
  }

  /** Puts the divider back where it started, and forgets the width: nothing has been chosen. */
  async function resetSidebarWidth(): Promise<void> {
    sidebarWidth.value = null;
    await persist();
  }

  /**
   * Puts the sidebar away, or brings it back.
   *
   * Written down at once rather than at the end of a drag: it is one decision, and somebody who
   * closed the sidebar and quit meant it.
   */
  async function setSidebarOpen(open: boolean): Promise<void> {
    sidebarOpen.value = open;
    await persist();
  }

  async function setLocale(next: SupportedLocale): Promise<void> {
    localeChosen.value = true;
    locale.value = next;
    apply();
    await persist();
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
    for (const field of fields) {
      switch (field) {
        case "themeMode":
          themeMode.value = FRESH.themeMode;
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
        case "locale":
          localeChosen.value = false;
          locale.value = resolveLocale(typeof navigator === "undefined" ? undefined : navigator.language);
          break;
      }
    }
    apply();
    await persist();
  }

  /** Applies the environment's preference and starts following the system. */
  function initialize(): () => void {
    const stopFollowing = followSystemTheme();
    apply();
    return stopFollowing;
  }

  return {
    themeMode,
    terminalTheme,
    filesLayout,
    sessionLayout,
    terminalPanelHeight,
    sidebarWidth,
    sidebarOpen,
    monitorEnabled,
    packedTransfer,
    resolvedTheme,
    resolvedTerminalTheme,
    resolvedPanelHeight,
    resolvedSidebarWidth,
    locale,
    problem,
    initialize,
    load,
    setThemeMode,
    setTerminalTheme,
    setFilesLayout,
    setSessionLayout,
    setMonitorEnabled,
    setPackedTransfer,
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
