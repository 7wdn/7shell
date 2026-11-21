/**
 * Reading a colour scheme somebody brought with them.
 *
 * The readers cover what circulates: the Xresources block terminals have used for decades, the
 * JSON Windows Terminal and most editors export (with VS Code's prefixed keys and nested
 * containers read too), iTerm2's plist, and the config dialects of the terminals people run —
 * Kitty, Ghostty, Alacritty, Warp, WezTerm and Konsole. Each reader recognises its own shape and
 * refuses what is not it, so the dispatcher can simply try them in order. The reading is
 * deliberately forgiving about which spelling a key uses — `cursor`, `cursorColor` and
 * `Cursor Color` are the same answer — and deliberately strict about the colours: a scheme that
 * cannot supply all sixteen is refused whole rather than padded with guesses.
 *
 * Nothing here touches the network, a path or the filesystem: the caller pastes text, this turns
 * it into schemes, and the settings file keeps the result.
 */

/** A scheme as it arrives, before the build assigns it an id. */
export interface ImportedTerminalScheme {
  name: string;
  foreground: string;
  background: string;
  cursor: string;
  /** ANSI SGR 0–15, normalized to `#rrggbb`. */
  colors: readonly string[];
  selectionBackground?: string;
}

export type ImportedSchemeFormat =
  | "json"
  | "iterm2"
  | "xresources"
  | "vscode"
  | "kitty"
  | "ghostty"
  | "alacritty"
  | "warp"
  | "wezterm"
  | "konsole"
  | "unknown";

export interface SchemeImportResult {
  schemes: readonly ImportedTerminalScheme[];
  format: ImportedSchemeFormat;
}

const ANSI_KEYS = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightblack",
  "brightred",
  "brightgreen",
  "brightyellow",
  "brightblue",
  "brightmagenta",
  "brightcyan",
  "brightwhite",
] as const;

/**
 * `#rgb` or `#rrggbb`, lower-case and with the short form expanded; anything else is `null`.
 * Kitty also documents `#rrrrggggbbbb` — each channel is four hex digits — and folds down to a
 * byte, which is reading the documented form rather than guessing.
 */
export function normalizeSchemeColour(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  const trimmed = value.trim().toLowerCase();
  if (/^#[0-9a-f]{6}$/.test(trimmed)) {
    return trimmed;
  }
  if (/^#[0-9a-f]{3}$/.test(trimmed)) {
    return `#${trimmed[1]}${trimmed[1]}${trimmed[2]}${trimmed[2]}${trimmed[3]}${trimmed[3]}`;
  }
  if (/^#[0-9a-f]{12}$/.test(trimmed)) {
    const channel = (start: number): string =>
      Math.round(Number.parseInt(trimmed.slice(start, start + 4), 16) / 257)
        .toString(16)
        .padStart(2, "0");
    return `#${channel(1)}${channel(5)}${channel(9)}`;
  }
  return null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** A field under any of its names, compared without case or punctuation. */
function field(record: Record<string, unknown>, ...names: readonly string[]): unknown {
  const wanted = new Set(names.map((name) => name.toLowerCase().replace(/[^a-z0-9]/g, "")));
  for (const [key, value] of Object.entries(record)) {
    if (wanted.has(key.toLowerCase().replace(/[^a-z0-9]/g, ""))) {
      return value;
    }
  }
  return undefined;
}

function colour(record: Record<string, unknown>, ...names: readonly string[]): string | null {
  return normalizeSchemeColour(field(record, ...names));
}

/** The eight colour names every config dialect spells the same, mapped to their slot in the first half. */
const ANSI_SLOTS: Record<string, number> = {
  black: 0,
  red: 1,
  green: 2,
  yellow: 3,
  blue: 4,
  magenta: 5,
  cyan: 6,
  white: 7,
};

/**
 * The colour spellings the config dialects use beyond JSON's `#rrggbb`: quotes around the value,
 * `0x…`, and the bare hex Ghostty and friends write. Anything that is not a colour stays `null`
 * and the slot it belonged to keeps its refusal.
 */
function configColour(value: unknown): string | null {
  if (typeof value !== "string") {
    return null;
  }
  let text = value
    .trim()
    .replace(/^["']|["']$/g, "")
    .trim();
  if (/^0x[0-9a-f]{3,12}$/i.test(text)) {
    text = `#${text.slice(2)}`;
  } else if (/^[0-9a-f]{3,12}$/i.test(text)) {
    text = `#${text}`;
  }
  return normalizeSchemeColour(text);
}

/** Builds the one importable scheme from parsed parts, or refuses the whole thing when a colour is missing. */
function assembleScheme(parts: {
  fallbackName: string;
  name?: string | null;
  foreground: string | null;
  background: string | null;
  cursor?: string | null;
  colors: readonly (string | null)[];
  selectionBackground?: string | null;
}): ImportedTerminalScheme | null {
  const { foreground, background, colors } = parts;
  if (!foreground || !background || colors.length !== 16 || colors.some((entry) => entry === null)) {
    return null;
  }
  return {
    name: (parts.name ?? "").trim() || parts.fallbackName,
    foreground,
    background,
    cursor: parts.cursor ?? foreground,
    colors: colors as readonly string[],
    selectionBackground: parts.selectionBackground ?? undefined,
  };
}

function coloursFromRecord(record: Record<string, unknown>): readonly string[] | null {
  const listed = field(record, "colors", "colours", "palette", "ansi");
  if (Array.isArray(listed) && listed.length === 16) {
    const normalized = listed.map(normalizeSchemeColour);
    return normalized.every((entry): entry is string => entry !== null) ? normalized : null;
  }
  const named = ANSI_KEYS.map((key) => normalizeSchemeColour(field(record, key)));
  return named.every((entry): entry is string => entry !== null) ? named : null;
}

function schemeFromRecord(record: Record<string, unknown>, fallbackName: string): ImportedTerminalScheme | null {
  const foreground = colour(record, "foreground", "fg");
  const background = colour(record, "background", "bg");
  const cursor = colour(record, "cursor", "cursorColor") ?? foreground;
  const colors = coloursFromRecord(record);
  if (!foreground || !background || !cursor || !colors) {
    return null;
  }
  const name = field(record, "name", "schemeName");
  return {
    name: typeof name === "string" && name.trim() ? name.trim() : fallbackName,
    foreground,
    background,
    cursor,
    colors,
    selectionBackground: colour(record, "selectionBackground", "selection", "selectionColor") ?? undefined,
  };
}

/**
 * VS Code keeps terminal colours one level down — under a theme's `colors` or the user's
 * `workbench.colorCustomizations` — and spells every key with `terminal.`/`ansi` prefixes. The
 * prefixes are noise: the words underneath are the ones every other format uses, so each lookup
 * is tried with and without them. `terminalCursor.foreground` folds into the cursor slot the
 * same way.
 */
function schemeFromVsCode(record: Record<string, unknown>, fallbackName: string): ImportedTerminalScheme | null {
  // settings.json spells the key with a dot at the top level; a wrapper object nests it. The
  // theme-file shape puts the same keys under `colors`.
  const workbench = field(record, "workbench");
  const customizations = isRecord(workbench)
    ? field(workbench, "colorCustomizations")
    : field(record, "workbench.colorCustomizations");
  const container = isRecord(customizations)
    ? customizations
    : isRecord(field(record, "colors"))
      ? (field(record, "colors") as Record<string, unknown>)
      : null;
  if (!container) {
    return null;
  }
  const lookup = (...names: readonly string[]): string | null => {
    for (const name of names) {
      for (const variant of [name, `terminal.${name}`, `ansi.${name}`, `terminal.ansi.${name}`]) {
        const value = field(container, variant);
        if (value === undefined) {
          continue;
        }
        const parsed = normalizeSchemeColour(value);
        if (parsed) {
          return parsed;
        }
      }
    }
    return null;
  };
  const colors = ANSI_KEYS.map((key) => lookup(key));
  const rawName = field(record, "name", "schemeName");
  return assembleScheme({
    fallbackName,
    name: typeof rawName === "string" ? rawName : null,
    foreground: lookup("foreground", "fg"),
    background: lookup("background", "bg"),
    cursor: lookup("cursor.foreground", "cursor", "cursorColor"),
    colors,
    selectionBackground: lookup("selectionBackground", "selection"),
  });
}

function parseJson(text: string, fallbackName: string): { schemes: ImportedTerminalScheme[]; vscode: boolean } {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return { schemes: [], vscode: false };
  }
  const candidates = Array.isArray(parsed)
    ? parsed
    : isRecord(parsed) && Array.isArray(parsed.schemes)
      ? parsed.schemes
      : [parsed];
  const schemes: ImportedTerminalScheme[] = [];
  let vscode = false;
  for (const candidate of candidates) {
    if (!isRecord(candidate)) {
      continue;
    }
    const scheme = schemeFromRecord(candidate, fallbackName);
    if (scheme) {
      schemes.push(scheme);
      continue;
    }
    const fromVsCode = schemeFromVsCode(candidate, fallbackName);
    if (fromVsCode) {
      vscode = true;
      schemes.push(fromVsCode);
    }
  }
  return { schemes, vscode };
}

/** iTerm2 stores each colour as three 0–1 components inside a `<dict>`. */
function itermComponent(body: string, name: string): number | null {
  const match = new RegExp(`<key>\\s*${name}\\s*Component\\s*</key>\\s*<real>\\s*([0-9.]+)\\s*</real>`, "i").exec(body);
  if (!match) {
    return null;
  }
  const value = Number(match[1]);
  return Number.isFinite(value) ? Math.min(255, Math.max(0, Math.round(value * 255))) : null;
}

function itermColour(body: string): string | null {
  const red = itermComponent(body, "Red");
  const green = itermComponent(body, "Green");
  const blue = itermComponent(body, "Blue");
  if (red === null || green === null || blue === null) {
    return null;
  }
  const hex = (value: number): string => value.toString(16).padStart(2, "0");
  return `#${hex(red)}${hex(green)}${hex(blue)}`;
}

function parseIterm(text: string, fallbackName: string): ImportedTerminalScheme[] {
  if (!/<plist|<dict/i.test(text)) {
    return [];
  }
  const entries = new Map<string, string>();
  const pattern = /<key>\s*([^<]+?)\s*<\/key>\s*<dict>([\s\S]*?)<\/dict>/gi;
  for (const match of text.matchAll(pattern)) {
    const key = match[1]!.toLowerCase().replace(/\s+/g, " ");
    const body = match[2]!;
    const parsed = itermColour(body);
    if (parsed) {
      entries.set(key, parsed);
    }
  }
  const colors = Array.from({ length: 16 }, (_, index) => entries.get(`ansi ${index} color`) ?? null);
  if (colors.some((entry) => entry === null)) {
    return [];
  }
  const foreground = entries.get("foreground color") ?? null;
  const background = entries.get("background color") ?? null;
  if (!foreground || !background) {
    return [];
  }
  return [
    {
      name: fallbackName,
      foreground,
      background,
      cursor: entries.get("cursor color") ?? foreground,
      colors: colors as string[],
      selectionBackground: entries.get("selection color") ?? undefined,
    },
  ];
}

function parseXresources(text: string, fallbackName: string): ImportedTerminalScheme[] {
  if (!/^\s*[^\n:]+\.(?:foreground|background|color0)\s*:/im.test(text)) {
    return [];
  }
  const entries = new Map<string, string>();
  let name: string | null = null;
  for (const line of text.split(/\r?\n/)) {
    const match = /^\s*[^:\s]+\.([A-Za-z0-9]+)\s*:\s*(.+?)\s*$/.exec(line);
    if (!match) {
      continue;
    }
    const key = match[1]!.toLowerCase();
    if (key === "name") {
      name = match[2]!;
      continue;
    }
    const parsed = normalizeSchemeColour(match[2]);
    if (parsed) {
      entries.set(key, parsed);
    }
  }
  const colors = Array.from({ length: 16 }, (_, index) => entries.get(`color${index}`) ?? null);
  const foreground = entries.get("foreground") ?? null;
  const background = entries.get("background") ?? null;
  if (colors.some((entry) => entry === null) || !foreground || !background) {
    return [];
  }
  return [
    {
      name: name?.trim() || fallbackName,
      foreground,
      background,
      cursor: entries.get("cursorcolor") ?? foreground,
      colors: colors as string[],
      selectionBackground: entries.get("selectionbackground") ?? undefined,
    },
  ];
}

/**
 * Kitty's conf: `key value` lines with no separator. `color0`…`color15` carry the palette and
 * the surfaces are plain words; colours may be the wide `#rrrrggggbbbb` form the normalizer
 * already folds. Anything kitty also configures — `url_color`, `foreground_opacity` — spells a
 * different key and never reaches the map.
 */
function parseKitty(text: string, fallbackName: string): ImportedTerminalScheme[] {
  const entries = new Map<string, string>();
  for (const line of text.split(/\r?\n/)) {
    const match = /^\s*(color\d+|foreground|background|cursor|selection_background)\s+(\S+)\s*$/.exec(line);
    if (!match) {
      continue;
    }
    const parsed = configColour(match[2]!);
    if (parsed) {
      entries.set(match[1]!, parsed);
    }
  }
  const colors = Array.from({ length: 16 }, (_, index) => entries.get(`color${index}`) ?? null);
  const scheme = assembleScheme({
    fallbackName,
    foreground: entries.get("foreground") ?? null,
    background: entries.get("background") ?? null,
    cursor: entries.get("cursor") ?? null,
    colors,
    selectionBackground: entries.get("selection_background") ?? null,
  });
  return scheme ? [scheme] : [];
}

/**
 * Ghostty's conf: `key = value` lines and `palette = N=#hex` for the sixteen slots. Themes in
 * the wild write the hex without the `#`, which `configColour` restores, so both spellings read.
 */
function parseGhostty(text: string, fallbackName: string): ImportedTerminalScheme[] {
  const entries = new Map<string, string>();
  const colors: (string | null)[] = Array.from({ length: 16 }, () => null);
  for (const line of text.split(/\r?\n/)) {
    const palette = /^\s*palette\s*=\s*(\d+)\s*=\s*(\S+)\s*$/.exec(line);
    if (palette) {
      const slot = Number.parseInt(palette[1]!, 10);
      const parsed = configColour(palette[2]!);
      if (slot >= 0 && slot < 16 && parsed && colors[slot] === null) {
        colors[slot] = parsed;
      }
      continue;
    }
    const named = /^\s*(background|foreground|cursor-color|selection-background)\s*=\s*(\S+)\s*$/.exec(line);
    if (named) {
      const parsed = configColour(named[2]!);
      if (parsed) {
        entries.set(named[1]!, parsed);
      }
    }
  }
  const scheme = assembleScheme({
    fallbackName,
    foreground: entries.get("foreground") ?? null,
    background: entries.get("background") ?? null,
    cursor: entries.get("cursor-color") ?? null,
    colors,
    selectionBackground: entries.get("selection-background") ?? null,
  });
  return scheme ? [scheme] : [];
}

/**
 * Warp's theme YAML: root `foreground`/`background`/`cursor`, and the sixteen colours indented
 * under `terminal_colors:` → `normal:`/`bright:`. A hand-read subset is enough because nothing
 * else in the file nests that deep: any line back at column zero ends the section, so a
 * `background_image:` block after it cannot be mistaken for colours.
 */
function parseWarp(text: string, fallbackName: string): ImportedTerminalScheme[] {
  const root = new Map<string, string>();
  const colors: (string | null)[] = Array.from({ length: 16 }, () => null);
  let section: "normal" | "bright" | null = null;
  for (const line of text.split(/\r?\n/)) {
    const atRoot = /^\S/.test(line);
    if (atRoot) {
      section = null;
      const named = /^([a-z_]+)\s*:\s*(.*?)\s*$/.exec(line);
      if (named && named[1] === "name") {
        root.set("name", named[2]!);
      } else if (named && ["foreground", "background", "cursor"].includes(named[1]!)) {
        const parsed = configColour(named[2]!);
        if (parsed) {
          root.set(named[1]!, parsed);
        }
      }
      continue;
    }
    const block = /^\s*(normal|bright)\s*:\s*$/.exec(line);
    if (block) {
      section = block[1] as "normal" | "bright";
      continue;
    }
    if (!section) {
      continue;
    }
    const colourEntry = /^\s*([a-z]+)\s*:\s*["']?([^"'\s]+)["']?\s*$/.exec(line);
    if (!colourEntry) {
      continue;
    }
    const slot = (ANSI_SLOTS[colourEntry[1]!] ?? -1) + (section === "bright" ? 8 : 0);
    const parsed = configColour(colourEntry[2]!);
    if (slot >= 0 && slot < 16 && parsed && colors[slot] === null) {
      colors[slot] = parsed;
    }
  }
  const scheme = assembleScheme({
    fallbackName,
    name: root.get("name") ?? null,
    foreground: root.get("foreground") ?? null,
    background: root.get("background") ?? null,
    cursor: root.get("cursor") ?? null,
    colors,
  });
  return scheme ? [scheme] : [];
}

/**
 * Alacritty's TOML: `[colors.primary]` for the two surfaces, `[colors.cursor]`,
 * `[colors.selection]`, and the named colours under `[colors.normal]` and `[colors.bright]`.
 * Reading those sections line by line is enough — any other table resets the section, so a
 * `[font]` block cannot bleed in. Values may still be the quoted or `0x…` spellings.
 */
function parseAlacritty(text: string, fallbackName: string): ImportedTerminalScheme[] {
  const colors: (string | null)[] = Array.from({ length: 16 }, () => null);
  const entries = new Map<string, string>();
  let section: string | null = null;
  for (const line of text.split(/\r?\n/)) {
    const heading = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (heading) {
      const path = heading[1]!.trim().toLowerCase();
      section = path === "colors" ? "" : path.startsWith("colors.") ? path.slice("colors.".length) : null;
      continue;
    }
    if (!section) {
      continue;
    }
    const pair = /^\s*([a-z_]+)\s*=\s*(.+?)\s*$/.exec(line);
    const parsed = pair ? configColour(pair[2]!) : null;
    if (!pair || !parsed) {
      continue;
    }
    if (section === "normal" || section === "bright") {
      const slot = ANSI_SLOTS[pair[1]!];
      if (slot !== undefined && colors[slot + (section === "bright" ? 8 : 0)] === null) {
        colors[slot + (section === "bright" ? 8 : 0)] = parsed;
      }
      continue;
    }
    entries.set(`${section}.${pair[1]!}`, parsed);
  }
  const scheme = assembleScheme({
    fallbackName,
    foreground: entries.get("primary.foreground") ?? null,
    background: entries.get("primary.background") ?? null,
    cursor: entries.get("cursor.cursor") ?? null,
    colors,
    selectionBackground: entries.get("selection.background") ?? null,
  });
  return scheme ? [scheme] : [];
}

/**
 * WezTerm's published TOML: `[colors]` with `ansi`/`brights` as the two eight-colour arrays and
 * the surfaces as plain keys. The older Lua export wraps the same names in `return { … }` with
 * braces, so both bracket styles are read — the names are what identify the format, not the
 * wrapper.
 */
function parseWezTerm(text: string, fallbackName: string): ImportedTerminalScheme[] {
  const half = (name: string): readonly (string | null)[] => {
    const match = new RegExp(`\\b${name}\\s*=\\s*[{\\[]([\\s\\S]*?)[}\\]]`).exec(text);
    return match ? match[1]!.split(",").map((part) => configColour(part)) : [];
  };
  const entries = new Map<string, string>();
  for (const match of text.matchAll(/^\s*(foreground|background|cursor_bg|selection_bg)\s*=\s*(.+?)\s*$/gim)) {
    const parsed = configColour(match[2]!);
    if (parsed && !entries.has(match[1]!)) {
      entries.set(match[1]!, parsed);
    }
  }
  const scheme = assembleScheme({
    fallbackName,
    foreground: entries.get("foreground") ?? null,
    background: entries.get("background") ?? null,
    cursor: entries.get("cursor_bg") ?? null,
    colors: [...half("ansi"), ...half("brights")],
    selectionBackground: entries.get("selection_bg") ?? null,
  });
  return scheme ? [scheme] : [];
}

/**
 * Konsole's .colorscheme: INI sections with decimal `R,G,B` components. `[ColorN]` is slot N for
 * N 0–7 (N 8–15 name the bright half directly); `[ColorNIntense]` is the bright variant of N.
 * The faint sections repeat the normal colours and are ignored, as are `[ForegroundIntense]` and
 * friends — the flat colour is the one a palette needs. Konsole has no cursor colour, so the
 * foreground stands in.
 */
function parseKonsole(text: string, fallbackName: string): ImportedTerminalScheme[] {
  const colors: (string | null)[] = Array.from({ length: 16 }, () => null);
  const entries = new Map<string, string>();
  let name: string | null = null;
  let section = "";
  for (const line of text.split(/\r?\n/)) {
    const heading = /^\s*\[([^\]]+)\]\s*$/.exec(line);
    if (heading) {
      section = heading[1]!.trim();
      continue;
    }
    const pair = /^\s*([A-Za-z]+)\s*=\s*(.+?)\s*$/.exec(line);
    if (!pair) {
      continue;
    }
    const [key, value] = [pair[1]!, pair[2]!];
    if (section === "General" && key === "Description") {
      name = value;
      continue;
    }
    if (key !== "Color") {
      continue;
    }
    const colour = konsoleColour(value);
    if (!colour) {
      continue;
    }
    const colorSection = /^Color(\d+)(Intense)?$/.exec(section);
    if (colorSection) {
      const slot = Number.parseInt(colorSection[1]!, 10) + (colorSection[2] ? 8 : 0);
      if (slot < 16 && colors[slot] === null) {
        colors[slot] = colour;
      }
    } else if (section === "Foreground") {
      entries.set("foreground", colour);
    } else if (section === "Background") {
      entries.set("background", colour);
    }
  }
  const scheme = assembleScheme({
    fallbackName,
    name,
    foreground: entries.get("foreground") ?? null,
    background: entries.get("background") ?? null,
    colors,
  });
  return scheme ? [scheme] : [];
}

/** Konsole spells colours as decimal components, optionally with a trailing alpha slot. */
function konsoleColour(value: string): string | null {
  const match = /^\s*(\d{1,3})\s*,\s*(\d{1,3})\s*,\s*(\d{1,3})(?:\s*,\s*\d{1,3})?\s*$/.exec(value);
  if (!match) {
    return null;
  }
  const channel = (text: string): string => Math.min(255, Number.parseInt(text, 10)).toString(16).padStart(2, "0");
  return `#${channel(match[1]!)}${channel(match[2]!)}${channel(match[3]!)}`;
}

/** Reads whichever format the text is, and says which one that was. */
export function parseTerminalSchemeImport(text: string, fallbackName = "Imported scheme"): SchemeImportResult {
  const trimmed = text.trim();
  if (!trimmed) {
    return { schemes: [], format: "unknown" };
  }
  const json = parseJson(trimmed, fallbackName);
  if (json.schemes.length > 0) {
    return { schemes: json.schemes, format: json.vscode ? "vscode" : "json" };
  }
  const readers: readonly (readonly [
    ImportedSchemeFormat,
    (text: string, fallbackName: string) => ImportedTerminalScheme[],
  ])[] = [
    ["iterm2", parseIterm],
    ["xresources", parseXresources],
    ["kitty", parseKitty],
    ["ghostty", parseGhostty],
    ["warp", parseWarp],
    ["alacritty", parseAlacritty],
    ["wezterm", parseWezTerm],
    ["konsole", parseKonsole],
  ];
  for (const [format, read] of readers) {
    const schemes = read(trimmed, fallbackName);
    if (schemes.length > 0) {
      return { schemes, format };
    }
  }
  return { schemes: [], format: "unknown" };
}

/** An imported scheme with the id the catalogue will know it by. */
export interface ImportedSchemeWithId extends ImportedTerminalScheme {
  id: string;
}

/**
 * Gives each parsed scheme an id nobody owns, up to the number of imported schemes this build keeps.
 *
 * `existingCount` is the number already imported, not the size of the catalogue: the shipped
 * schemes do not count against the limit. Counting the wrong set is a bug that looks like an import
 * doing nothing, because the catalogue is already over the limit on its own.
 */
export function mergeImportedSchemes(
  parsed: readonly ImportedTerminalScheme[],
  takenIds: ReadonlySet<string>,
  existingCount: number,
  limit: number,
): ImportedSchemeWithId[] {
  const taken = new Set(takenIds);
  const next: ImportedSchemeWithId[] = [];
  let kept = existingCount;
  for (const scheme of parsed) {
    if (kept >= limit) {
      break;
    }
    const id = importedSchemeId(scheme.name, taken);
    taken.add(id);
    next.push({ id, ...scheme });
    kept += 1;
  }
  return next;
}

/** A stable id from a name, made unique against the ids already in use. */
export function importedSchemeId(name: string, taken: ReadonlySet<string>): string {
  const base =
    name
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "scheme";
  if (!taken.has(base)) {
    return base;
  }
  for (let suffix = 2; ; suffix += 1) {
    const candidate = `${base}-${suffix}`;
    if (!taken.has(candidate)) {
      return candidate;
    }
  }
}
