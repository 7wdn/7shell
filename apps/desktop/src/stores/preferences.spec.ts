import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({ appInfo: vi.fn(), settingsGet: vi.fn(), settingsSet: vi.fn() }));

import { i18n } from "@/i18n";
import { settingsGet, settingsSet } from "@/lib/backend/tauri";
import { DEFAULT_TERMINAL_PANEL_HEIGHT, usePreferencesStore } from "@/stores/preferences";
import type { AppSettings } from "@/types/app";

const settingsGetMock = vi.mocked(settingsGet);
const settingsSetMock = vi.mocked(settingsSet);

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
    terminalTheme: "system",
    filesLayout: "tree",
    sessionLayout: "split",
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
      terminalTheme: "system",
      filesLayout: "tree",
      sessionLayout: "split",
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
      terminalTheme: "system",
      filesLayout: "tree",
      sessionLayout: "split",
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
      terminalTheme: "light",
      filesLayout: "tree",
      sessionLayout: "split",
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

    await store.reset(["themeMode", "terminalTheme", "filesLayout", "sessionLayout", "packedTransfer"]);

    expect(store.themeMode).toBe("system");
    expect(store.terminalTheme).toBe("system");
    expect(store.filesLayout).toBe("tree");
    expect(store.sessionLayout).toBe("split");
    expect(store.packedTransfer).toBe(false);
    expect(settingsSetMock).toHaveBeenCalledTimes(1);
    expect(settingsSetMock).toHaveBeenLastCalledWith({
      themeMode: "system",
      terminalTheme: "system",
      filesLayout: "tree",
      sessionLayout: "split",
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
