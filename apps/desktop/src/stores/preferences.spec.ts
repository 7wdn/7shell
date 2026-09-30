import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({ appInfo: vi.fn(), settingsGet: vi.fn(), settingsSet: vi.fn() }));

import { i18n } from "@/i18n";
import { settingsGet, settingsSet } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";

const settingsGetMock = vi.mocked(settingsGet);
const settingsSetMock = vi.mocked(settingsSet);

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
    settingsGetMock.mockReset().mockResolvedValue({ themeMode: "system" });
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
    settingsGetMock.mockResolvedValue({ themeMode: "light", locale: "zh-CN" });

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

    expect(settingsSetMock).toHaveBeenCalledWith({ themeMode: "dark", locale: undefined });
  });

  it("records the language only once someone chooses one", async () => {
    const store = usePreferencesStore();
    store.initialize();
    await store.load();

    await store.setLocale("zh-CN");

    expect(settingsSetMock).toHaveBeenCalledWith({ themeMode: "system", locale: "zh-CN" });
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
