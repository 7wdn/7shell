/**
 * Locale files are JSON, one per domain, nested under the domain key, so a checker script can
 * read them without parsing TypeScript (`scripts/check-i18n.mjs` verifies that every key used by
 * Rust, the descriptors and the interface exists in every locale).
 */

import { createI18n } from "vue-i18n";

import enApp from "./locales/en/app.json";
import enAuth from "./locales/en/auth.json";
import enCapability from "./locales/en/capability.json";
import enCommon from "./locales/en/common.json";
import enErrors from "./locales/en/errors.json";
import enHome from "./locales/en/home.json";
import enProtocol from "./locales/en/protocol.json";
import enSettings from "./locales/en/settings.json";
import enShell from "./locales/en/shell.json";
import zhCnApp from "./locales/zh-CN/app.json";
import zhCnAuth from "./locales/zh-CN/auth.json";
import zhCnCapability from "./locales/zh-CN/capability.json";
import zhCnCommon from "./locales/zh-CN/common.json";
import zhCnErrors from "./locales/zh-CN/errors.json";
import zhCnHome from "./locales/zh-CN/home.json";
import zhCnProtocol from "./locales/zh-CN/protocol.json";
import zhCnSettings from "./locales/zh-CN/settings.json";
import zhCnShell from "./locales/zh-CN/shell.json";

export const SUPPORTED_LOCALES = ["en", "zh-CN"] as const;

export type SupportedLocale = (typeof SUPPORTED_LOCALES)[number];

/** English is the default and the fallback: a missing translation is a bug, not a mode. */
export const DEFAULT_LOCALE: SupportedLocale = "en";

export function isSupportedLocale(value: string): value is SupportedLocale {
  return (SUPPORTED_LOCALES as readonly string[]).includes(value);
}

/** Picks the closest supported locale for a browser language tag such as `zh-Hans-CN`. */
export function resolveLocale(tag: string | undefined): SupportedLocale {
  if (!tag) {
    return DEFAULT_LOCALE;
  }

  const normalized = tag.toLowerCase();
  const exact = SUPPORTED_LOCALES.find((locale) => locale.toLowerCase() === normalized);
  if (exact) {
    return exact;
  }

  // Fall back to the language subtag, so `zh-Hans-CN` and `zh-TW` both land on `zh-CN`:
  // for someone who reads Chinese, Simplified is a better answer than English.
  const language = normalized.split("-")[0] ?? "";
  if (language === "") {
    return DEFAULT_LOCALE;
  }
  return SUPPORTED_LOCALES.find((locale) => (locale.split("-")[0] ?? "").toLowerCase() === language) ?? DEFAULT_LOCALE;
}

const messages = {
  en: {
    ...enCommon,
    ...enShell,
    ...enHome,
    ...enSettings,
    ...enApp,
    ...enErrors,
    ...enProtocol,
    ...enAuth,
    ...enCapability,
  },
  "zh-CN": {
    ...zhCnCommon,
    ...zhCnShell,
    ...zhCnHome,
    ...zhCnSettings,
    ...zhCnApp,
    ...zhCnErrors,
    ...zhCnProtocol,
    ...zhCnAuth,
    ...zhCnCapability,
  },
};

export const i18n = createI18n({
  legacy: false,
  locale: DEFAULT_LOCALE,
  fallbackLocale: DEFAULT_LOCALE,
  messages,
});
