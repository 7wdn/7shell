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
  | "global.toggleTransfers"
  | "global.toggleAssistant"
  | "terminal.find"
  | "terminal.splitRight"
  | "terminal.splitDown"
  | "terminal.focusNextPane"
  | "terminal.focusPreviousPane"
  | "terminal.zoomPane"
  | "terminal.closePane"
  | "files.focusPath"
  | "files.filter"
  | "commands.completionAccept"
  | "commands.completionTrigger"
  | "editor.save"
  | "editor.submit"
  | "editor.cancel"
  | "editor.toggleTabFocus";

export interface ShortcutDefinition {
  readonly id: ShortcutActionId;
  readonly scope: ShortcutScope;
  /** What the action is called, as a key: the settings page is where a person reads it. */
  readonly labelKey: string;
  /** `Mod` is Ctrl everywhere and Command on macOS; see `binding.ts`. */
  readonly defaultBinding: string;
}

/** The surfaces a binding can belong to. */
export type ShortcutScope = "global" | "terminal" | "files" | "commands" | "editor";

/** The surfaces in the order the settings page lists them. */
export const SHORTCUT_SCOPE_ORDER: readonly ShortcutScope[] = ["global", "terminal", "files", "commands", "editor"];

export const SHORTCUT_DEFINITIONS: readonly ShortcutDefinition[] = [
  {
    id: "global.uploadClipboard",
    scope: "global",
    labelKey: "settings.shortcuts.actions.uploadClipboard",
    defaultBinding: "Mod+V",
  },
  // Shifted on purpose: every plain control combination belongs to the terminal — Ctrl+J is a newline
  // and Ctrl+D ends its input — and this panel has to be openable while somebody is typing in one.
  {
    id: "global.toggleTransfers",
    scope: "global",
    labelKey: "settings.shortcuts.actions.toggleTransfers",
    defaultBinding: "Mod+Shift+D",
  },
  // Shifted for the same reason: the assistant panel has to open while somebody is typing into a
  // terminal, and every plain control combination below belongs to the shell on the far side.
  {
    id: "global.toggleAssistant",
    scope: "global",
    labelKey: "settings.shortcuts.actions.toggleAssistant",
    defaultBinding: "Mod+Shift+A",
  },
  {
    id: "terminal.find",
    scope: "terminal",
    labelKey: "settings.shortcuts.actions.terminalFind",
    defaultBinding: "Mod+F",
  },
  {
    id: "terminal.splitRight",
    scope: "terminal",
    labelKey: "settings.shortcuts.actions.splitRight",
    defaultBinding: "Mod+Shift+E",
  },
  {
    id: "terminal.splitDown",
    scope: "terminal",
    labelKey: "settings.shortcuts.actions.splitDown",
    defaultBinding: "Mod+Shift+O",
  },
  {
    id: "terminal.focusNextPane",
    scope: "terminal",
    labelKey: "settings.shortcuts.actions.focusNextPane",
    defaultBinding: "Mod+Shift+]",
  },
  {
    id: "terminal.focusPreviousPane",
    scope: "terminal",
    labelKey: "settings.shortcuts.actions.focusPreviousPane",
    defaultBinding: "Mod+Shift+[",
  },
  {
    id: "terminal.zoomPane",
    scope: "terminal",
    labelKey: "settings.shortcuts.actions.zoomPane",
    defaultBinding: "Mod+Shift+Z",
  },
  {
    id: "terminal.closePane",
    scope: "terminal",
    labelKey: "settings.shortcuts.actions.closePane",
    defaultBinding: "Mod+Shift+X",
  },
  // What a file manager and a browser both use for their address bar. It is scoped to the panel,
  // so the shell on the far side keeps Ctrl+L for clearing its screen.
  {
    id: "files.focusPath",
    scope: "files",
    labelKey: "settings.shortcuts.actions.focusPath",
    defaultBinding: "Mod+L",
  },
  // The same key as the terminal's find, in the surface where the rows are what is being looked
  // through. Scoping is what keeps the two from both answering: a terminal's key belongs to the
  // terminal, and this one only reaches the panel that has the keyboard.
  {
    id: "files.filter",
    scope: "files",
    labelKey: "settings.shortcuts.actions.filterFiles",
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
  {
    id: "editor.save",
    scope: "editor",
    labelKey: "settings.shortcuts.actions.editorSave",
    defaultBinding: "Mod+S",
  },
  {
    id: "editor.submit",
    scope: "editor",
    labelKey: "settings.shortcuts.actions.editorSubmit",
    defaultBinding: "Mod+Enter",
  },
  {
    id: "editor.cancel",
    scope: "editor",
    labelKey: "settings.shortcuts.actions.editorCancel",
    defaultBinding: "Escape",
  },
  {
    id: "editor.toggleTabFocus",
    scope: "editor",
    labelKey: "settings.shortcuts.actions.editorTabFocus",
    defaultBinding: "Mod+Shift+M",
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
