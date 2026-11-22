/**
 * Enough colour arithmetic to preview the minimum-contrast setting.
 *
 * The renderer raises any text colour whose contrast against its background falls below the
 * setting, blending it towards white on a dark background and towards black on a light one. The
 * preview needs the same answer without a renderer, so this re-derives it from the WCAG
 * relative-luminance definition: the same ratio the renderer targets, blended in the same
 * direction, in small steps so a colour stops as soon as it clears the bar. An approximation on
 * purpose — close enough that a slider drag shows the same colours the pane will draw, not a
 * second implementation to keep in lockstep with the renderer's.
 *
 * The sources are the three spellings this part of the interface actually meets: the hex that
 * schemes and rules are written in, the `rgb()` a computed style answers with, and the `oklch()`
 * the palette tokens are declared in.
 */

interface Rgb {
  readonly r: number;
  readonly g: number;
  readonly b: number;
}

const BYTE_MAX = 255;

function clampByte(value: number): number {
  return Math.min(BYTE_MAX, Math.max(0, Math.round(value)));
}

function toHex(rgb: Rgb): string {
  const channel = (value: number): string => clampByte(value).toString(16).padStart(2, "0");
  return `#${channel(rgb.r)}${channel(rgb.g)}${channel(rgb.b)}`;
}

/** The sRGB transfer function, shared by the luminance definition and the oklch decode. */
function gammaEncode(value: number): number {
  const clamped = Math.min(1, Math.max(0, value));
  return clamped <= 0.0031308 ? clamped * 12.92 : 1.055 * clamped ** (1 / 2.4) - 0.055;
}

function gammaDecode(value: number): number {
  const clamped = Math.min(1, Math.max(0, value));
  return clamped <= 0.04045 ? clamped / 12.92 : ((clamped + 0.055) / 1.055) ** 2.4;
}

/** WCAG relative luminance, the number the contrast ratio is defined over. */
function relativeLuminance(rgb: Rgb): number {
  return (
    0.2126 * gammaDecode(rgb.r / BYTE_MAX) +
    0.7152 * gammaDecode(rgb.g / BYTE_MAX) +
    0.0722 * gammaDecode(rgb.b / BYTE_MAX)
  );
}

/** The WCAG contrast ratio of two colours, 1 (identical) to 21 (black on white). */
export function contrastRatio(first: string, second: string): number | null {
  const a = parsePreviewColour(first);
  const b = parsePreviewColour(second);
  if (!a || !b) {
    return null;
  }
  const one = relativeLuminance(a) + 0.05;
  const two = relativeLuminance(b) + 0.05;
  return Math.max(one, two) / Math.min(one, two);
}

/** oklch → OKLab → the LMS midpoint the OKLab paper gives, then linear sRGB and the gamma back. */
function oklchToRgb(lightness: number, chroma: number, hueDegrees: number): Rgb {
  const hue = (hueDegrees * Math.PI) / 180;
  const a = chroma * Math.cos(hue);
  const b = chroma * Math.sin(hue);
  const l_ = lightness + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = lightness - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = lightness - 0.0894841775 * a - 1.291485548 * b;
  const l = l_ ** 3;
  const m = m_ ** 3;
  const s = s_ ** 3;
  return {
    r: clampByte(gammaEncode(4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s) * BYTE_MAX),
    g: clampByte(gammaEncode(-1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s) * BYTE_MAX),
    b: clampByte(gammaEncode(-0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s) * BYTE_MAX),
  };
}

/** Reads any of the three spellings; a colour this cannot read is one it leaves alone. */
export function parsePreviewColour(value: string): Rgb | null {
  const text = value.trim().toLowerCase();
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/.exec(text);
  if (hex) {
    const digits = hex[1]!;
    const full = digits.length === 3 ? digits.replace(/./g, "$&$&") : digits;
    return {
      r: Number.parseInt(full.slice(0, 2), 16),
      g: Number.parseInt(full.slice(2, 4), 16),
      b: Number.parseInt(full.slice(4, 6), 16),
    };
  }
  const fn = /^rgba?\(\s*(\d{1,3})\s*[, ]\s*(\d{1,3})\s*[, ]\s*(\d{1,3})/.exec(text);
  if (fn) {
    return { r: clampByte(Number(fn[1])), g: clampByte(Number(fn[2])), b: clampByte(Number(fn[3])) };
  }
  const oklch = /^oklch\(\s*([\d.]+)%?\s+([\d.]+)\s+(-?[\d.]+)/.exec(text);
  if (oklch) {
    const lightness = oklch[1]!.includes("%") ? Number(oklch[1]!.replace("%", "")) / 100 : Number(oklch[1]);
    return oklchToRgb(lightness, Number(oklch[2]), Number(oklch[3]));
  }
  return null;
}

function blend(rgb: Rgb, target: Rgb, amount: number): Rgb {
  return {
    r: rgb.r + (target.r - rgb.r) * amount,
    g: rgb.g + (target.g - rgb.g) * amount,
    b: rgb.b + (target.b - rgb.b) * amount,
  };
}

const WHITE: Rgb = { r: BYTE_MAX, g: BYTE_MAX, b: BYTE_MAX };
const BLACK: Rgb = { r: 0, g: 0, b: 0 };
const BLEND_STEPS = 32;

/**
 * The colour to draw when the setting demands more contrast than the pair has.
 *
 * Unchanged — as written, whatever spelling came in — when the pair already clears the bar, or
 * when either colour is one this cannot read: refusing to guess is what keeps a broken value from
 * becoming a broken drawing. Adjusted colours come back as hex, the one spelling a style attribute
 * always takes.
 */
export function ensureMinimumContrast(colour: string, background: string, minimum: number): string {
  const source = parsePreviewColour(colour);
  const base = parsePreviewColour(background);
  if (!source || !base || minimum <= 1 || contrastRatio(colour, background)! >= minimum) {
    return colour;
  }
  // The direction the renderer moves: away from the background's own brightness, so light text on
  // dark grows lighter and dark text on light grows darker.
  const target = relativeLuminance(base) < 0.5 ? WHITE : BLACK;
  for (let step = 1; step <= BLEND_STEPS; step += 1) {
    const blended = blend(source, target, step / BLEND_STEPS);
    if (contrastRatio(toHex(blended), background)! >= minimum) {
      return toHex(blended);
    }
  }
  return toHex(source);
}
