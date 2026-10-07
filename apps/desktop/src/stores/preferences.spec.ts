import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  appInfo: vi.fn(),
  settingsGet: vi.fn(),
  settingsSet: vi.fn(),
  webviewSetZoom: vi.fn(),
}));

import { i18n } from "@/i18n";
import { settingsGet, settingsSet, webviewSetZoom } from "@/lib/backend/tauri";
import { DEFAULT_TERMINAL_PANEL_HEIGHT, usePreferencesStore } from "@/stores/preferences";
import {
  DEFAULT_CORNER_STYLE,
  DEFAULT_CUSTOM_COLORS,
  DEFAULT_CUSTOM_COLORS_DARK,
  DEFAULT_TERMINAL_FONT,
  DEFAULT_UI_FONT,
  DEFAULT_UI_SCALE,
  type AppSettings,
} from "@/types/app";

const settingsGetMock = vi.mocked(settingsGet);
const settingsSetMock = vi.mocked(settingsSet);
const webviewSetZoomMock = vi.mocked(webviewSetZoom);

/**
 * What the backend answers when nothing has been chosen.
 *
 * Every field, because the wire type has them all: the settings file may leave one out, but what
 * the interface is handed is always a whole `AppSettings`. A mock that leaves a field out is a
 * mock that disagrees with the type, and the store would read the disagreement as a choice.
 */
function defaults(overrides: Partial<AppSettings> = {}): AppSettings {
  return {
    themeMode: "system",
    themePalette: "default",
    uiScale: DEFAULT_UI_SCALE,
    cornerStyle: DEFAULT_CORNER_STYLE,
    uiFont: undefined,
    terminalFont: undefined,
    customColors: DEFAULT_CUSTOM_COLORS,
    customColorsDark: DEFAULT_CUSTOM_COLORS_DARK,
    terminalTheme: "system",
    filesLayout: "tree",
    sessionLayout: "split",
    sidebarOpen: true,
    monitorEnabled: true,
    packedTransfer: false,
    ...overrides,
  };
}

const NEWER_SCHEMA = {
  version: 1,
  code: "STORE-SCHEMA-NEWER",
  messageKey: "errors.store.schemaNewer",
  messageParams: { found: "2", supported: "1" },
  origin: { subsystem: "store" },
  outcome: "rejected",
};

/** A controllable stand-in for the operating system's colour preference. */
function stubSystemTheme(initiallyDark: boolean) {
  const listeners = new Set<(event: MediaQueryListEvent) => void>();
  const query = {
    matches: initiallyDark,
    media: "(prefers-color-scheme: dark)",
    addEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => listeners.add(listener),
    removeEventListener: (_type: string, listener: (event: MediaQueryListEvent) => void) => listeners.delete(listener),
  };
  vi.stubGlobal("matchMedia", () => query);

  return {
    listenerCount: () => listeners.size,
    setDark(value: boolean) {
      query.matches = value;
      for (const listener of listeners) {
        listener({ matches: value } as MediaQueryListEvent);
      }
    },
  };
}

describe("preferences store", () => {
  let system: ReturnType<typeof stubSystemTheme>;

  beforeEach(() => {
    setActivePinia(createPinia());
    system = stubSystemTheme(true);
    settingsGetMock.mockReset().mockResolvedValue(defaults());
    settingsSetMock.mockReset().mockResolvedValue(undefined);
    document.documentElement.classList.remove("dark");
  });

  it("follows the system when nothing has been chosen", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.themeMode).toBe("system");
    expect(store.resolvedTheme).toBe("dark");
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("keeps following the system while it is open", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    system.setDark(false);

    expect(store.resolvedTheme).toBe("light");
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("applies a stored choice instead of the system", async () => {
    settingsGetMock.mockResolvedValue(defaults({ themeMode: "light", locale: "zh-CN" }));

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.themeMode).toBe("light");
    expect(store.resolvedTheme).toBe("light");
    expect(store.locale).toBe("zh-CN");
    expect(i18n.global.locale.value).toBe("zh-CN");
    expect(document.documentElement.lang).toBe("zh-CN");
  });

  it("stops following the system once a theme is chosen", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setThemeMode("light");
    system.setDark(true);

    expect(store.resolvedTheme).toBe("light");
  });

  it("records a theme choice and leaves the language unset", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setThemeMode("dark");

    expect(settingsSetMock).toHaveBeenCalledWith({
      themeMode: "dark",
      themePalette: "default",
      uiScale: DEFAULT_UI_SCALE,
      cornerStyle: DEFAULT_CORNER_STYLE,
      uiFont: undefined,
      terminalFont: undefined,
      customColors: DEFAULT_CUSTOM_COLORS,
      customColorsDark: DEFAULT_CUSTOM_COLORS_DARK,
      terminalTheme: "system",
      filesLayout: "tree",
      sessionLayout: "split",
      sidebarWidth: undefined,
      sidebarOpen: true,
      monitorEnabled: true,
      packedTransfer: false,
      locale: undefined,
    });
  });

  it("records the language only once someone chooses one", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setLocale("zh-CN");

    expect(settingsSetMock).toHaveBeenCalledWith({
      themeMode: "system",
      themePalette: "default",
      uiScale: DEFAULT_UI_SCALE,
      cornerStyle: DEFAULT_CORNER_STYLE,
      uiFont: undefined,
      terminalFont: undefined,
      customColors: DEFAULT_CUSTOM_COLORS,
      customColorsDark: DEFAULT_CUSTOM_COLORS_DARK,
      terminalTheme: "system",
      filesLayout: "tree",
      sessionLayout: "split",
      sidebarWidth: undefined,
      sidebarOpen: true,
      monitorEnabled: true,
      packedTransfer: false,
      locale: "zh-CN",
    });
    expect(document.documentElement.lang).toBe("zh-CN");
  });

  it("opens anyway when the settings file cannot be read", async () => {
    settingsGetMock.mockRejectedValue(NEWER_SCHEMA);

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.problem?.code).toBe("STORE-SCHEMA-NEWER");
    // The defaults still applied, so the window is usable and the reason is visible.
    expect(store.resolvedTheme).toBe("dark");
  });

  it("surfaces a failed write without losing the choice", async () => {
    settingsSetMock.mockRejectedValue("the disk is full");

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setThemeMode("light");

    expect(store.themeMode).toBe("light");
    expect(store.resolvedTheme).toBe("light");
    expect(store.problem?.code).toBe("APP-INTERNAL");
  });

  it("stops listening to the system when the interface goes away", async () => {
    const store = usePreferencesStore();
    const stopFollowing = store.initialize();
    await store.load();

    expect(system.listenerCount()).toBe(1);
    stopFollowing();
    expect(system.listenerCount()).toBe(0);
  });
});

describe("the terminal palette", () => {
  let system: ReturnType<typeof stubSystemTheme>;

  beforeEach(() => {
    setActivePinia(createPinia());
    system = stubSystemTheme(true);
    settingsGetMock.mockReset().mockResolvedValue(defaults());
    settingsSetMock.mockReset().mockResolvedValue(undefined);
  });

  it("follows the interface by default, not the operating system", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    // The interface is dark because the system is dark; the terminal follows the interface.
    expect(store.resolvedTheme).toBe("dark");
    expect(store.resolvedTerminalTheme).toBe("dark");

    // The system turns light, the interface follows it, and the terminal follows the interface.
    system.setDark(false);
    expect(store.resolvedTheme).toBe("light");
    expect(store.resolvedTerminalTheme).toBe("light");

    // An interface chosen by hand carries the terminal with it too.
    await store.setThemeMode("dark");
    expect(store.resolvedTerminalTheme).toBe("dark");
  });

  it("keeps a palette the person chose, whatever the interface does", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setTerminalTheme("light");
    expect(store.resolvedTerminalTheme).toBe("light");

    await store.setThemeMode("dark");
    // The interface is dark again; the terminal keeps the palette it was given.
    expect(store.resolvedTheme).toBe("dark");
    expect(store.resolvedTerminalTheme).toBe("light");
    expect(settingsSetMock).toHaveBeenLastCalledWith({
      themeMode: "dark",
      themePalette: "default",
      uiScale: DEFAULT_UI_SCALE,
      cornerStyle: DEFAULT_CORNER_STYLE,
      uiFont: undefined,
      terminalFont: undefined,
      customColors: DEFAULT_CUSTOM_COLORS,
      customColorsDark: DEFAULT_CUSTOM_COLORS_DARK,
      terminalTheme: "light",
      filesLayout: "tree",
      sessionLayout: "split",
      sidebarWidth: undefined,
      sidebarOpen: true,
      monitorEnabled: true,
      packedTransfer: false,
      locale: undefined,
    });
  });

  it("remembers the choice across a restart", async () => {
    settingsGetMock.mockResolvedValue(defaults({ themeMode: "light", terminalTheme: "dark" }));

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.resolvedTerminalTheme).toBe("dark");
  });
});

describe("the colour theme", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    stubSystemTheme(true);
    settingsGetMock.mockReset().mockResolvedValue(defaults());
    settingsSetMock.mockReset().mockResolvedValue(undefined);
    delete document.documentElement.dataset.palette;
  });

  it("wears the palette the file names, on top of the theme the file names", async () => {
    settingsGetMock.mockResolvedValue(defaults({ themeMode: "dark", themePalette: "sage" }));

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.themePalette).toBe("sage");
    expect(document.documentElement.dataset.palette).toBe("sage");
    // The palette is a tint over the theme rather than a third theme: it says nothing about
    // whether the lights are on, and the theme's own answer is untouched.
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("changes the colour without changing whether the lights are on", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setThemePalette("claude");

    expect(document.documentElement.dataset.palette).toBe("claude");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ themePalette: "claude" }));
  });

  it("wears a colour the pointer is on without choosing it", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    store.previewThemePalette("cobalt");

    expect(document.documentElement.dataset.palette, "the window shows it").toBe("cobalt");
    expect(store.themePalette, "and nothing has been chosen").toBe("default");
    // A preview is not a choice, so it is not a write either: the file keeps what was chosen until
    // somebody chooses something else.
    expect(settingsSetMock).not.toHaveBeenCalled();
  });

  it("puts the chosen colour back when the pointer leaves the list", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    store.previewThemePalette("cobalt");
    store.clearThemePalettePreview();

    expect(document.documentElement.dataset.palette).toBe("default");
    expect(store.wornPalette).toBe("default");
  });

  it("stops previewing the moment the colour is actually chosen", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    store.previewThemePalette("cobalt");
    await store.setThemePalette("cobalt");

    // The choice and the preview are the same value from here on, which is what makes letting go of
    // the list afterwards a no-op rather than a jump back to what used to be chosen.
    expect(store.wornPalette).toBe("cobalt");
    store.clearThemePalettePreview();
    expect(document.documentElement.dataset.palette).toBe("cobalt");
  });

  it("shows the chosen colour again when the pointer comes back over it", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    store.previewThemePalette("cobalt");
    // Back onto the option that is actually chosen: there is nothing left to preview, so what is on
    // screen is the choice again — which is what somebody comparing two colours is looking for.
    store.previewThemePalette("default");

    expect(document.documentElement.dataset.palette).toBe("default");
    expect(store.themePalette).toBe("default");
  });

  it("writes a hand-picked palette as tokens on the document, and takes them back", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setThemePalette("custom");

    // The window's own tokens, for the theme on screen…
    expect(document.documentElement.style.getPropertyValue("--background")).toBe("rgb(19 20 22)");
    // …and both halves of the pane's, because the terminal draws in whichever it was told to and
    // that is not always the theme of the window around it.
    expect(document.documentElement.style.getPropertyValue("--terminal-dark-background")).toBe("rgb(25 25 28)");
    expect(document.documentElement.style.getPropertyValue("--terminal-light-background")).toBe("rgb(250 250 250)");

    await store.setThemePalette("default");

    // Named palettes are the stylesheet's again: the inline tokens are removed rather than
    // overwritten, or the last hand-picked colour would go on winning over every palette after it.
    expect(document.documentElement.style.getPropertyValue("--background")).toBe("");
    expect(document.documentElement.style.getPropertyValue("--terminal-dark-background")).toBe("");
  });

  it("changes one colour of the set the theme on screen is in", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();
    await store.setThemePalette("custom");

    await store.setCustomColor("primary", "#4f9cf0");

    expect(document.documentElement.style.getPropertyValue("--primary")).toBe("rgb(79 156 240)");
    expect(store.customColorsDark.primary).toBe("#4f9cf0");
    expect(store.customColors.primary, "the light set is the other half of the palette").toBe("#171717");
  });

  it("puts the hand-picked colours back to what a fresh install starts from", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();
    await store.setThemePalette("custom");
    await store.setCustomColor("background", "#101820");

    await store.resetCustomColors();

    expect(store.customColorsDark.background).toBe(DEFAULT_CUSTOM_COLORS_DARK.background);
    expect(document.documentElement.style.getPropertyValue("--background")).toBe("rgb(19 20 22)");
    expect(settingsSetMock).toHaveBeenLastCalledWith(
      expect.objectContaining({ customColorsDark: DEFAULT_CUSTOM_COLORS_DARK }),
    );
  });

  it("writes the corner style on the document, and puts it back", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    // The roundness is a handful of numbers the stylesheet declares; what the store does is say
    // which set, and an attribute is how it says it — the same handover the palette uses.
    expect(document.documentElement.dataset.cornerStyle).toBe("large");

    await store.setCornerStyle("none");

    expect(document.documentElement.dataset.cornerStyle).toBe("none");
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ cornerStyle: "none" }));

    await store.reset(["cornerStyle"]);
    expect(store.cornerStyle).toBe(DEFAULT_CORNER_STYLE);
    expect(document.documentElement.dataset.cornerStyle).toBe("large");
  });

  it("writes both stacks onto the document, and shows one before it is chosen", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    // Nothing chosen still means a family is being drawn in: the stylesheet's declaration is
    // written over with the same value, so what the document says and what the store says agree.
    expect(document.documentElement.style.getPropertyValue("--font-sans")).toBe(DEFAULT_UI_FONT);

    await store.setUiFont("'Fira Code', monospace");

    expect(document.documentElement.style.getPropertyValue("--font-sans")).toBe("'Fira Code', monospace");
    expect(store.wornTerminalFont, "the terminal's own choice is not the interface's font").toBe(DEFAULT_TERMINAL_FONT);
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ uiFont: "'Fira Code', monospace" }));
  });

  it("shows a family the pointer is on without choosing it", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    store.previewFont("terminalFont", "'Cascadia Code', monospace");

    expect(store.wornTerminalFont, "the pane is handed this").toBe("'Cascadia Code', monospace");
    expect(store.wornUiFont, "the other list is not the preview").toBe(DEFAULT_UI_FONT);
    expect(document.documentElement.style.getPropertyValue("--font-sans"), "and the interface is untouched").toBe(
      DEFAULT_UI_FONT,
    );
    expect(store.terminalFont, "and nothing has been chosen").toBeNull();
    expect(settingsSetMock).not.toHaveBeenCalled();

    store.clearFontPreview();
    expect(store.wornTerminalFont).toBe(DEFAULT_TERMINAL_FONT);
  });

  it("stops previewing the moment a family is chosen", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    store.previewFont("uiFont", "'Consolas', monospace");
    await store.setUiFont("'Consolas', monospace");

    // Letting go of the list afterwards is a no-op rather than a jump back to what used to be
    // chosen: the preview and the choice are the same stack from here on.
    store.clearFontPreview();
    expect(document.documentElement.style.getPropertyValue("--font-sans")).toBe("'Consolas', monospace");
  });
});

describe("the interface scale", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    stubSystemTheme(true);
    settingsGetMock.mockReset().mockResolvedValue(defaults());
    settingsSetMock.mockReset().mockResolvedValue(undefined);
    webviewSetZoomMock.mockReset().mockResolvedValue(undefined);
  });

  it("draws the interface at the size the last run was left at", async () => {
    settingsGetMock.mockResolvedValue(defaults({ uiScale: 125 }));

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.uiScale).toBe(125);
    // The file holds a percentage and the shell takes a factor: the conversion happens here, once.
    expect(webviewSetZoomMock).toHaveBeenCalledWith(1.25);
  });

  it("asks the window for the new size, and writes it down", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();
    webviewSetZoomMock.mockClear();

    await store.setUiScale(150);

    expect(webviewSetZoomMock).toHaveBeenCalledWith(1.5);
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ uiScale: 150 }));
  });

  it("does not ask again for a size the window already has", async () => {
    // Applying runs whenever the system's own preference moves, and the scale has nothing to do
    // with that: a window following the operating system must not become a call per change.
    const store = usePreferencesStore();
    store.initialize();
    await store.load();
    webviewSetZoomMock.mockClear();

    await store.setThemeMode("light");

    expect(webviewSetZoomMock).not.toHaveBeenCalled();
  });

  it("says so when the window refuses the size, rather than looking stuck at the old one", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();
    webviewSetZoomMock.mockRejectedValue(new Error("the window said no"));

    await store.setUiScale(150);

    await vi.waitFor(() => expect(store.problem?.code).toBe("APP-INTERNAL"));
  });
});

describe("the file layout", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    settingsGetMock.mockReset().mockResolvedValue(defaults());
    settingsSetMock.mockReset().mockResolvedValue(undefined);
  });

  it("starts on the tree, because that is what a remote file system is easiest to move around in", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.filesLayout).toBe("tree");
  });

  it("remembers a choice of the single list", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setFilesLayout("list");

    expect(store.filesLayout).toBe("list");
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ filesLayout: "list" }));
  });

  it("reads the choice back on the next start", async () => {
    settingsGetMock.mockResolvedValue(defaults({ filesLayout: "list" }));

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.filesLayout).toBe("list");
  });
});

describe("how a session opens", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    settingsGetMock.mockReset().mockResolvedValue(defaults());
    settingsSetMock.mockReset().mockResolvedValue(undefined);
  });

  it("opens both halves by default, so the files are there without asking", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.sessionLayout).toBe("split");
  });

  it("remembers a choice of the terminal alone", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setSessionLayout("terminal");

    expect(store.sessionLayout).toBe("terminal");
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ sessionLayout: "terminal" }));
  });

  it("reads the choice back on the next start", async () => {
    settingsGetMock.mockResolvedValue(defaults({ sessionLayout: "terminal" }));

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.sessionLayout).toBe("terminal");
  });
});

describe("the host monitor", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    settingsGetMock.mockReset().mockResolvedValue(defaults());
    settingsSetMock.mockReset().mockResolvedValue(undefined);
  });

  it("is on until somebody turns it off", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.monitorEnabled).toBe(true);
  });

  it("remembers being hidden, because hiding it is also what stops the sampling", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setMonitorEnabled(false);

    expect(store.monitorEnabled).toBe(false);
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ monitorEnabled: false }));
  });

  it("reads the choice back on the next start", async () => {
    settingsGetMock.mockResolvedValue(defaults({ monitorEnabled: false }));

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.monitorEnabled).toBe(false);
  });
});

describe("the packed-transfer switch", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    settingsGetMock.mockReset().mockResolvedValue(defaults());
    settingsSetMock.mockReset().mockResolvedValue(undefined);
  });

  it("is off until somebody turns it on, because it changes what lands on the disk", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.packedTransfer).toBe(false);
  });

  it("writes the choice down, so the menu and the dialog cannot disagree", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setPackedTransfer(true);

    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ packedTransfer: true }));
  });

  it("reads the choice back on the next start", async () => {
    settingsGetMock.mockResolvedValue(defaults({ packedTransfer: true }));

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.packedTransfer).toBe(true);
  });
});

describe("the file panel under a terminal", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    settingsGetMock.mockReset().mockResolvedValue(defaults());
    settingsSetMock.mockReset().mockResolvedValue(undefined);
  });

  it("starts at a height that leaves the terminal the larger half", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.resolvedPanelHeight).toBe(DEFAULT_TERMINAL_PANEL_HEIGHT);
  });

  it("draws the height being dragged to before it writes anything down", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    // A pointer lands on fractions; a pane is drawn at whole pixels.
    store.setPanelHeight(412.4);

    expect(store.resolvedPanelHeight).toBe(412);
    // A drag is dozens of moves and every write is a synced file, so nothing is written yet.
    expect(settingsSetMock).not.toHaveBeenCalled();
  });

  it("writes the height down when the drag is let go", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    store.setPanelHeight(412);
    await store.rememberPanelHeight();

    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ terminalPanelHeight: 412 }));
  });

  it("reads the height back on the next start", async () => {
    settingsGetMock.mockResolvedValue(defaults({ terminalPanelHeight: 360 }));

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    expect(store.resolvedPanelHeight).toBe(360);
  });

  it("leaves nothing behind in the file when the divider is put back where it started", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    store.setPanelHeight(500);
    await store.resetPanelHeight();

    expect(store.resolvedPanelHeight).toBe(DEFAULT_TERMINAL_PANEL_HEIGHT);
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ terminalPanelHeight: undefined }));
  });
});

describe("restoring the defaults", () => {
  const CHOSEN = defaults({
    themeMode: "dark",
    themePalette: "jetbrains",
    uiScale: 150,
    terminalTheme: "light",
    filesLayout: "list",
    sessionLayout: "terminal",
    monitorEnabled: false,
    packedTransfer: true,
    locale: "zh-CN",
  });

  beforeEach(() => {
    setActivePinia(createPinia());
    settingsGetMock.mockReset().mockResolvedValue(CHOSEN);
    settingsSetMock.mockReset().mockResolvedValue(undefined);
  });

  // The one test below stands in for the system's language, and nothing else in the file should
  // inherit that stand-in.
  afterEach(() => vi.unstubAllGlobals());

  it("puts the named fields back to what a fresh install has, in one write", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.reset([
      "themeMode",
      "themePalette",
      "uiScale",
      "terminalTheme",
      "filesLayout",
      "sessionLayout",
      "packedTransfer",
    ]);

    expect(store.themeMode).toBe("system");
    expect(store.themePalette).toBe("default");
    expect(store.uiScale).toBe(DEFAULT_UI_SCALE);
    expect(store.terminalTheme).toBe("system");
    expect(store.filesLayout).toBe("tree");
    expect(store.sessionLayout).toBe("split");
    expect(store.packedTransfer).toBe(false);
    expect(settingsSetMock).toHaveBeenCalledTimes(1);
    expect(settingsSetMock).toHaveBeenLastCalledWith({
      themeMode: "system",
      themePalette: "default",
      uiScale: DEFAULT_UI_SCALE,
      cornerStyle: DEFAULT_CORNER_STYLE,
      uiFont: undefined,
      terminalFont: undefined,
      customColors: DEFAULT_CUSTOM_COLORS,
      customColorsDark: DEFAULT_CUSTOM_COLORS_DARK,
      terminalTheme: "system",
      filesLayout: "tree",
      sessionLayout: "split",
      sidebarWidth: undefined,
      sidebarOpen: true,
      monitorEnabled: false,
      packedTransfer: false,
      locale: "zh-CN",
    });
  });

  it("leaves the fields it was not asked about where they were", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.reset(["filesLayout"]);

    expect(store.filesLayout).toBe("tree");
    expect(store.monitorEnabled).toBe(false);
    expect(store.packedTransfer).toBe(true);
    expect(store.locale).toBe("zh-CN");
  });

  it("takes the language back to following the system rather than to a language of its own", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();
    vi.stubGlobal("navigator", { language: "en-GB" });

    await store.reset(["locale"]);

    expect(store.locale).toBe("en");
    expect(document.documentElement.lang).toBe("en");
    // Not written down: an absent locale is what leaves the system's language applying.
    expect(settingsSetMock).toHaveBeenLastCalledWith(expect.objectContaining({ locale: undefined }));
  });

  it("surfaces a write that fails, the same as any other choice", async () => {
    settingsSetMock.mockRejectedValue("the disk is full");

    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.reset(["themeMode"]);

    expect(store.themeMode).toBe("system");
    expect(store.problem?.code).toBe("APP-INTERNAL");
  });
});
