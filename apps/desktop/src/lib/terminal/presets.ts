/**
 * Ready-made answers to "which terminal colours".
 *
 * A preset is the same shape as a saved profile, so applying one and applying somebody's own are the
 * same path. The names that are proper nouns are not translated; the two that describe an answer
 * ("interface palette", "high contrast") carry keys the interface translates.
 */
import type { TerminalSchemeProfile } from "@/types/app";

export interface TerminalSchemePreset {
  readonly id: string;
  readonly name: string;
  /** A translation key for a name that is a description rather than a proper noun. */
  readonly nameKey?: string;
  readonly profile: TerminalSchemeProfile;
}

export const TERMINAL_SCHEME_PRESETS: readonly TerminalSchemePreset[] = [
  {
    id: "preset-interface",
    name: "Interface palette",
    nameKey: "settings.schemes.presets.interface",
    profile: {
      id: "preset-interface",
      name: "Interface palette",
      terminalTheme: "system",
      schemeSource: "host",
      schemeBackground: "scheme",
    },
  },
  {
    id: "preset-dracula",
    name: "Dracula",
    profile: {
      id: "preset-dracula",
      name: "Dracula",
      terminalTheme: "dark",
      schemeSource: "custom",
      schemeDark: "dracula",
      schemeLight: "solarized-light",
      schemeBackground: "scheme",
    },
  },
  {
    id: "preset-nord",
    name: "Nord",
    profile: {
      id: "preset-nord",
      name: "Nord",
      terminalTheme: "dark",
      schemeSource: "custom",
      schemeDark: "nord",
      schemeLight: "onehalflight",
      schemeBackground: "scheme",
    },
  },
  {
    id: "preset-tokyo-night",
    name: "Tokyo Night",
    profile: {
      id: "preset-tokyo-night",
      name: "Tokyo Night",
      terminalTheme: "dark",
      schemeSource: "custom",
      schemeDark: "tokyonight-storm",
      schemeLight: "tokyonight-day",
      schemeBackground: "scheme",
    },
  },
  {
    id: "preset-gruvbox",
    name: "Gruvbox",
    profile: {
      id: "preset-gruvbox",
      name: "Gruvbox",
      terminalTheme: "dark",
      schemeSource: "custom",
      schemeDark: "gruvbox-dark",
      schemeLight: "onehalflight",
      schemeBackground: "scheme",
    },
  },
  {
    id: "preset-high-contrast",
    name: "High contrast",
    nameKey: "settings.schemes.presets.highContrast",
    profile: {
      id: "preset-high-contrast",
      name: "High contrast",
      terminalTheme: "dark",
      schemeSource: "custom",
      schemeDark: "hardcore",
      schemeLight: "terminal-basic",
      schemeBackground: "scheme",
    },
  },
];
