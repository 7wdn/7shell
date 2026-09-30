import { defineStore } from "pinia";
import { ref } from "vue";

import { DEFAULT_LOCALE, type SupportedLocale, i18n, resolveLocale } from "@/i18n";

export type Theme = "dark" | "light";

/**
 * Language and theme.
 *
 * Deliberately not persisted yet: settings are stored by the backend once the vault
 * exists, so this interface never grows a second place to keep preferences.
 */
export const usePreferencesStore = defineStore("preferences", () => {
  const locale = ref<SupportedLocale>(DEFAULT_LOCALE);
  const theme = ref<Theme>("dark");

  function setLocale(next: SupportedLocale): void {
    locale.value = next;
    i18n.global.locale.value = next;
    document.documentElement.lang = next;
  }

  function setTheme(next: Theme): void {
    theme.value = next;
    document.documentElement.classList.toggle("dark", next === "dark");
  }

  function toggleTheme(): void {
    setTheme(theme.value === "dark" ? "light" : "dark");
  }

  /** Applies the environment's preference once, at startup. */
  function initialize(): void {
    setLocale(resolveLocale(typeof navigator === "undefined" ? undefined : navigator.language));
    setTheme(theme.value);
  }

  return { locale, theme, setLocale, setTheme, toggleTheme, initialize };
});
