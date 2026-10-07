/**
 * The two font settings: what a choice is worth, what the list offers, and what a name means.
 *
 * A font setting stores a whole `font-family` stack rather than one name, and that is the point
 * rather than an accident. What somebody picks is "draw in this face"; what gets written down is
 * that face followed by the fallbacks, so a name that turns out not to be installed — a font
 * imported on this machine and not on that one, a spelling that differs between platforms — leaves
 * a readable window rather than a browser default nobody chose.
 *
 * The list is built the same way round: what this build ships with, then the families that are
 * famous enough to name, then everything this machine actually has, and last whatever is currently
 * chosen in case it is a name typed by hand. A name typed by hand is taken as one, quoted, with
 * `monospace` after it.
 */
import { backend } from "@/lib/backend/api";
import { DEFAULT_TERMINAL_FONT, DEFAULT_UI_FONT, FONT_PRESETS, SYSTEM_UI_FONT } from "@/types/app";

/** How a family name has to be written inside a `font-family` stack. */
export function cssFontFamilyForName(name: string): string {
  return `'${name.replace(/\\/g, "\\\\").replace(/'/g, "\\'")}', monospace`;
}

/** The first family of a stack, without its quotes — what a person calls the face they chose. */
export function readableFontFamily(value: string): string {
  const first = value.split(",")[0]?.trim() ?? value;
  return first.replace(/^['"]|['"]$/g, "").replace(/\\'/g, "'");
}

/**
 * What to store when somebody types a name instead of picking one.
 *
 * A whole stack is taken as it stands — somebody who typed one meant it — and a bare name is quoted
 * and given a fallback, because a family name with a space in it is two families without the quotes.
 */
export function normalizeCustomFontFamilyInput(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) {
    return "";
  }
  if (trimmed.includes(",") || trimmed.includes("'") || trimmed.includes('"')) {
    return trimmed;
  }
  return cssFontFamilyForName(trimmed);
}

/**
 * The families offered for one of the two settings, in the order they are drawn.
 *
 * Four parts, in this order: what this build names first, the fixed-width families people arrive
 * with, every family the machine has, and — last — the family in force, so that a choice made on
 * another machine is still shown as the choice rather than as nothing. Deduplicated by stack, which
 * is the only kind of duplicate that can be made here: two ways of writing the same family name.
 *
 * The presets come before the machine's own list rather than being filtered against it, so a family
 * that is both — Consolas, on an ordinary Windows machine — is offered twice: once with the fallbacks
 * this build gives it, once as the machine spells it. That is left as it stands rather than deduped
 * by name: the two are different stacks, the list is one somebody searches by typing, and the row
 * that comes up first is the one that works when the family is missing.
 */
export function buildFontOptions(
  systemFonts: readonly string[],
  selected: readonly string[],
  leading: readonly string[],
): string[] {
  return [
    ...new Set([
      ...leading,
      ...FONT_PRESETS.map((preset) => preset.value),
      ...systemFonts.map(cssFontFamilyForName),
      ...selected.filter(Boolean),
    ]),
  ];
}

/**
 * What each of the two settings offers: the same list, led by different entries.
 *
 * The interface is led by the face the machine draws its own windows in and the family this build
 * ships — a person choosing here is usually asking for one of those two — and the terminal by the
 * family this build ships, because a terminal is read in a fixed-width face and the presets under it
 * are the fixed-width faces people arrive with.
 */
export function fontOptionsFor(
  family: "interface" | "terminal",
  systemFonts: readonly string[],
  selected: string,
): string[] {
  const leading = family === "interface" ? [SYSTEM_UI_FONT, DEFAULT_UI_FONT] : [DEFAULT_TERMINAL_FONT];
  return buildFontOptions(systemFonts, [selected], leading);
}

let cachedSystemFonts: string[] | null = null;
let pendingSystemFonts: Promise<string[]> | null = null;

/**
 * This machine's families, asked for once.
 *
 * The read is several hundred font files and the interface asks for it while a dropdown is opening,
 * so the answer is kept — and the *promise* is kept rather than the value, so that two pickers
 * opening at the same moment are one read rather than two.
 */
export async function loadSystemFontNames(): Promise<string[]> {
  if (cachedSystemFonts) {
    return cachedSystemFonts;
  }
  pendingSystemFonts ??= backend.fonts
    .systemList()
    .catch(() => [])
    .finally(() => {
      pendingSystemFonts = null;
    });
  cachedSystemFonts = await pendingSystemFonts;
  return cachedSystemFonts;
}
