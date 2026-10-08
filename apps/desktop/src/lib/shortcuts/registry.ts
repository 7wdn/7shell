/**
 * Every key combination this application reserves, in one table.
 *
 * One place to answer "what does this program use the keyboard for": a surface reads its combination
 * from here rather than spelling it out, so a change is one edit and two surfaces cannot drift apart.
 * A menu that advertises a key the application does not listen on is worse than a menu that
 * advertises nothing, and that is the failure this table exists to prevent.
 *
 * `scope` is where a binding is live: a terminal's find key means nothing in a file list. It is what
 * makes "this key is already taken" answerable, and what a rebinding page would group by.
 *
 * Nobody can rebind anything yet — the settings have no field for it. `resolveBindings` takes the
 * overrides anyway, so the day that field arrives there is one seam to read through instead of two.
 */

export type ShortcutActionId =
  | "global.uploadClipboard"
  | "terminal.find"
  | "commands.completionAccept"
  | "commands.completionTrigger";

export interface ShortcutDefinition {
  readonly id: ShortcutActionId;
  readonly scope: ShortcutScope;
  /** What the action is called, as a key: the settings page is where a person reads it. */
  readonly labelKey: string;
  /** `Mod` is Ctrl everywhere and Command on macOS; see `binding.ts`. */
  readonly defaultBinding: string;
}

/** The surfaces a binding can belong to. */
export type ShortcutScope = "global" | "terminal" | "files" | "commands";

/** The surfaces in the order the settings page lists them. */
export const SHORTCUT_SCOPE_ORDER: readonly ShortcutScope[] = ["global", "terminal", "files", "commands"];

export const SHORTCUT_DEFINITIONS: readonly ShortcutDefinition[] = [
  {
    id: "global.uploadClipboard",
    scope: "global",
    labelKey: "settings.shortcuts.actions.uploadClipboard",
    defaultBinding: "Mod+V",
  },
  {
    id: "terminal.find",
    scope: "terminal",
    labelKey: "settings.shortcuts.actions.terminalFind",
    defaultBinding: "Mod+F",
  },
  {
    id: "commands.completionAccept",
    scope: "commands",
    labelKey: "settings.shortcuts.actions.completionAccept",
    defaultBinding: "Tab",
  },
  // Not Ctrl+Space: on Windows that combination belongs to the input method, and a key that switches
  // the IME instead of opening the list is a key nobody can use.
  {
    id: "commands.completionTrigger",
    scope: "commands",
    labelKey: "settings.shortcuts.actions.completionTrigger",
    defaultBinding: "Alt+/",
  },
];

export type ShortcutOverrides = Partial<Record<ShortcutActionId, string>>;

/**
 * The bindings in force, defaults with whatever somebody has replaced.
 *
 * An override that is an empty string unbinds the action rather than falling back to the default:
 * "I do not want this key to do that" is a decision, and quietly restoring the default would ignore
 * it.
 */
export function resolveBindings(overrides: ShortcutOverrides = {}): Record<ShortcutActionId, string> {
  const resolved = {} as Record<ShortcutActionId, string>;
  for (const definition of SHORTCUT_DEFINITIONS) {
    resolved[definition.id] = overrides[definition.id] ?? definition.defaultBinding;
  }
  return resolved;
}

export function bindingFor(id: ShortcutActionId, overrides: ShortcutOverrides = {}): string {
  return resolveBindings(overrides)[id];
}
