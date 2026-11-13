/**
 * Reading a colour scheme somebody brought with them.
 *
 * Three formats cover nearly everything in circulation: the Xresources block terminals have used
 * for decades, the JSON Windows Terminal and most editors export, and iTerm2's plist. The reading
 * is deliberately forgiving about which spelling a key uses — `cursor`, `cursorColor` and
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

export type ImportedSchemeFormat = "json" | "iterm2" | "xresources" | "unknown";

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

/** `#rgb` or `#rrggbb`, lower-case and with the short form expanded; anything else is `null`. */
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

function parseJson(text: string, fallbackName: string): ImportedTerminalScheme[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return [];
  }
  const candidates = Array.isArray(parsed)
    ? parsed
    : isRecord(parsed) && Array.isArray(parsed.schemes)
      ? parsed.schemes
      : [parsed];
  const schemes: ImportedTerminalScheme[] = [];
  for (const candidate of candidates) {
    if (!isRecord(candidate)) {
      continue;
    }
    const scheme = schemeFromRecord(candidate, fallbackName);
    if (scheme) {
      schemes.push(scheme);
    }
  }
  return schemes;
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

/** Reads whichever of the three formats the text is, and says which one that was. */
export function parseTerminalSchemeImport(text: string, fallbackName = "Imported scheme"): SchemeImportResult {
  const trimmed = text.trim();
  if (!trimmed) {
    return { schemes: [], format: "unknown" };
  }
  const json = parseJson(trimmed, fallbackName);
  if (json.length > 0) {
    return { schemes: json, format: "json" };
  }
  const iterm = parseIterm(trimmed, fallbackName);
  if (iterm.length > 0) {
    return { schemes: iterm, format: "iterm2" };
  }
  const xresources = parseXresources(trimmed, fallbackName);
  if (xresources.length > 0) {
    return { schemes: xresources, format: "xresources" };
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
