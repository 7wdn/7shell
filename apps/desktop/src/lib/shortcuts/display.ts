/**
 * How a binding is spelled for a person.
 *
 * macOS gets the glyphs the system uses (`⌘ ⌥ ⇧ ⌃`) with a space between them; everyone else gets
 * `Ctrl + Alt + …`. What a menu or a tooltip prints comes from here, so the two surfaces cannot
 * disagree about what a key is called, and neither of them has to know what platform it is on.
 */
import { isMacPlatform, parseBinding, type ShortcutModifier } from "@/lib/shortcuts/binding";

const MAC_MODIFIER_GLYPHS: Readonly<Record<ShortcutModifier, string>> = {
  Mod: "⌘",
  Ctrl: "⌃",
  Alt: "⌥",
  Shift: "⇧",
  Meta: "⌘",
};

const OTHER_MODIFIER_NAMES: Readonly<Record<ShortcutModifier, string>> = {
  Mod: "Ctrl",
  Ctrl: "Ctrl",
  Alt: "Alt",
  Shift: "Shift",
  Meta: "Meta",
};

/** Named keys, spelled the way each platform writes them. Anything else is printed as it was read. */
const MAC_KEY_NAMES: Readonly<Record<string, string>> = {
  enter: "↩",
  escape: "⎋",
  tab: "⇥",
  backspace: "⌫",
  delete: "⌦",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
};

const OTHER_KEY_NAMES: Readonly<Record<string, string>> = {
  enter: "Enter",
  escape: "Esc",
  tab: "Tab",
  backspace: "Backspace",
  delete: "Delete",
  arrowup: "↑",
  arrowdown: "↓",
  arrowleft: "←",
  arrowright: "→",
};

function keyLabel(key: string, mac: boolean): string {
  if ([...key].length === 1) {
    return key.toUpperCase();
  }
  const names = mac ? MAC_KEY_NAMES : OTHER_KEY_NAMES;
  return names[key] ?? key;
}

/**
 * The binding as a person should read it.
 *
 * A binding nothing can parse is returned as it was written rather than swallowed: a menu that shows
 * a strange string is still telling the truth about what the table holds. An empty binding is empty —
 * an action somebody has unbound has no key to show.
 */
export function formatShortcutDisplay(binding: string, platform: string): string {
  const parsed = parseBinding(binding);
  if (parsed === null) {
    return binding.trim();
  }

  const mac = isMacPlatform(platform);
  const parts = parsed.modifiers.map((modifier) =>
    mac ? MAC_MODIFIER_GLYPHS[modifier] : OTHER_MODIFIER_NAMES[modifier],
  );
  parts.push(keyLabel(parsed.key, mac));
  return parts.join(mac ? " " : " + ");
}
