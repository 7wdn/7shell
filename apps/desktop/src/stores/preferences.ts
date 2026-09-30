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
import type { ResolvedTheme, ThemeMode } from "@/types/app";

const DARK_QUERY = "(prefers-color-scheme: dark)";

export const usePreferencesStore = defineStore("preferences", () => {
  const themeMode = ref<ThemeMode>("system");
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
      if (settings.locale && isSupportedLocale(settings.locale)) {
        locale.value = settings.locale;
        localeChosen.value = true;
      } else {
        locale.value = resolveLocale(typeof navigator === "undefined" ? undefined : navigator.language);
      }
    } catch (error) {
      // A settings file that cannot be read must not stop the interface from opening: the
      // defaults are used and the reason is surfaced in the settings dialog.
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

  async function setLocale(next: SupportedLocale): Promise<void> {
    localeChosen.value = true;
    locale.value = next;
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
    resolvedTheme,
    locale,
    problem,
    initialize,
    load,
    setThemeMode,
    setLocale,
  };
});
