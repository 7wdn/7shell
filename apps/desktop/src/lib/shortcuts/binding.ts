/**
 * Reading a key binding, and asking whether a keyboard event is it.
 *
 * A binding is one string — `Mod+V`, `Alt+/`, `Tab` — written the same way whatever platform it is
 * read on: `Mod` is Ctrl everywhere except macOS, where it is Command. The platform arrives as the
 * string the host reports (`macos aarch64`), so a table can be asked about both platforms without a
 * browser.
 *
 * Matching is exact: every modifier a binding does not name must be up, and every one it names must
 * be down. `Ctrl+Shift+F` is therefore not `Ctrl+F` — which is what keeps the interface's
 * combinations from swallowing the ones a shell or an editor means for itself.
 */

/** The modifiers a binding may name, in the order they are written. */
const MODIFIERS = ["Mod", "Ctrl", "Alt", "Shift", "Meta"] as const;

export type ShortcutModifier = (typeof MODIFIERS)[number];

export interface ParsedBinding {
  readonly modifiers: readonly ShortcutModifier[];
  /** Lowercased: `V` and `v` are the same key, and so are `ArrowUp` and `arrowup`. */
  readonly key: string;
}

/** The part of a keyboard event a binding is decided from. */
export interface ShortcutEventLike {
  readonly key: string;
  /** The physical key, when the event carries one. See `CODE_BY_CHARACTER`. */
  readonly code?: string;
  readonly ctrlKey: boolean;
  readonly altKey: boolean;
  readonly shiftKey: boolean;
  readonly metaKey: boolean;
}

export function isMacPlatform(platform: string): boolean {
  return platform.startsWith("macos");
}

/**
 * The physical key a character sits on.
 *
 * `event.key` is what the character *became*: on macOS `Alt+/` arrives as `÷`, and Alt+letters become
 * other letters on several European layouts. A binding that compared characters alone would be dead
 * there without saying so. `event.code` is the same key on every layout, so it is the second opinion.
 */
const CODE_BY_CHARACTER: Readonly<Record<string, string>> = {
  "/": "Slash",
  ".": "Period",
  ",": "Comma",
  ";": "Semicolon",
  "'": "Quote",
  "[": "BracketLeft",
  "]": "BracketRight",
  "\\": "Backslash",
  "-": "Minus",
  "=": "Equal",
  "`": "Backquote",
};

function characterCode(character: string): string | undefined {
  if (/^[a-z]$/.test(character)) {
    return `Key${character.toUpperCase()}`;
  }
  if (/^[0-9]$/.test(character)) {
    return `Digit${character}`;
  }
  return CODE_BY_CHARACTER[character];
}

/**
 * A binding, read into modifiers and a key.
 *
 * `null` for anything nothing can match: an empty string, or a name no modifier answers to. A typo
 * in a table is then a binding that never fires rather than one that fires on the wrong key.
 */
export function parseBinding(binding: string): ParsedBinding | null {
  // Split without dropping empty parts: `Mod+` ends on the separator and names no key at all, which
  // is a binding nothing can match rather than one for the `+` key.
  const parts = binding.trim().split("+");
  const key = (parts.pop() ?? "").trim();
  if (key === "") {
    return null;
  }

  const modifiers: ShortcutModifier[] = [];
  for (const part of parts) {
    const modifier = MODIFIERS.find((candidate) => candidate.toLowerCase() === part.trim().toLowerCase());
    if (modifier === undefined) {
      return null;
    }
    if (!modifiers.includes(modifier)) {
      modifiers.push(modifier);
    }
  }

  // In the table's order rather than the order somebody typed them: a binding is written one way, so
  // `Shift+Mod+K` and `Mod+Shift+K` are the same binding and are displayed the same way.
  modifiers.sort((left, right) => MODIFIERS.indexOf(left) - MODIFIERS.indexOf(right));

  return { modifiers, key: key.toLowerCase() };
}

export function matchesShortcut(event: ShortcutEventLike, binding: string, platform: string): boolean {
  const parsed = parseBinding(binding);
  if (parsed === null) {
    return false;
  }

  const mac = isMacPlatform(platform);
  // `Mod` and `Ctrl` (and `Meta` on macOS) are two names for one physical key, so they are resolved
  // per platform before the event is compared: on Windows a binding for `Mod+V` is satisfied by Ctrl
  // and must not be satisfied by the Windows key, and on macOS the same binding means Command alone.
  const wantsCtrl = parsed.modifiers.some((modifier) =>
    mac ? modifier === "Ctrl" : modifier === "Mod" || modifier === "Ctrl",
  );
  const wantsMeta = parsed.modifiers.some((modifier) =>
    mac ? modifier === "Mod" || modifier === "Meta" : modifier === "Meta",
  );
  const wantsAlt = parsed.modifiers.includes("Alt");
  const wantsShift = parsed.modifiers.includes("Shift");

  if (event.ctrlKey !== wantsCtrl || event.metaKey !== wantsMeta) {
    return false;
  }
  if (event.altKey !== wantsAlt || event.shiftKey !== wantsShift) {
    return false;
  }

  if (event.key.toLowerCase() === parsed.key) {
    return true;
  }
  return characterCode(parsed.key) === event.code;
}

/**
 * A binding written the one way this application stores it.
 *
 * The modifiers in the table's order and a single character upper-cased, so that two spellings of one
 * combination cannot sit in the settings file as if they were two keys. `null` for anything that
 * cannot be read at all.
 */
export function normalizeBinding(binding: string): string | null {
  const parsed = parseBinding(binding);
  if (parsed === null) {
    return null;
  }

  const parts = binding.trim().split("+");
  const raw = (parts.pop() ?? "").trim();
  const key = [...raw].length === 1 ? raw.toUpperCase() : raw;
  return [...parsed.modifiers, key].join("+");
}

/** The keys a press can be made of without naming one: a modifier on its own is not a combination. */
const MODIFIER_KEYS = new Set(["Shift", "Control", "Alt", "AltGraph", "Meta", "OS", "Hyper", "Super"]);

/**
 * The binding a key press stands for, in this application's spelling — which is what recording a new
 * key writes down.
 *
 * `Mod` is what is written for the platform's own modifier (Ctrl here, Command on a Mac): a stored
 * binding then means the same key on every platform, and the file does not have to be healed when it
 * travels between two of them. `null` for a press that names no key — a modifier on its own, or a
 * dead key.
 */
export function eventToBinding(event: ShortcutEventLike, platform: string): string | null {
  if (event.key === "" || MODIFIER_KEYS.has(event.key)) {
    return null;
  }

  const mac = isMacPlatform(platform);
  const modifiers: ShortcutModifier[] = [];
  if (mac ? event.metaKey : event.ctrlKey) {
    modifiers.push("Mod");
  }
  if (!mac && event.metaKey) {
    modifiers.push("Meta");
  }
  if (mac && event.ctrlKey) {
    modifiers.push("Ctrl");
  }
  if (event.altKey) {
    modifiers.push("Alt");
  }
  if (event.shiftKey) {
    modifiers.push("Shift");
  }

  modifiers.sort((left, right) => MODIFIERS.indexOf(left) - MODIFIERS.indexOf(right));
  const key = [...event.key].length === 1 ? event.key.toUpperCase() : event.key;
  return normalizeBinding([...modifiers, key].join("+"));
}
