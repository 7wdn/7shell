/**
 * Five colours in, a whole window out.
 *
 * The contract this file is about is readability, and it is not visible in a screenshot of any one
 * palette: somebody who picks a pale accent or a light foreground on a dark background is going to
 * get text they cannot read unless something decides the rest of the tokens for them. That is what
 * these cases hold — the surfaces move together, the text keeps its contrast, and the terminal is
 * dressed from the same five rather than left in the build's own greys.
 */
import { describe, expect, it } from "vitest";

import { deriveCustomColors, mix, readableTextOn } from "@/lib/settings/customColors";
import { DEFAULT_CUSTOM_COLORS, DEFAULT_CUSTOM_COLORS_DARK } from "@/types/app";

/** A colour's three channels, from either of the two shapes this file deals in: `rgb(…)` and hex. */
function channels(colour: string): number[] {
  if (colour.startsWith("#")) {
    return [1, 3, 5].map((at) => parseInt(colour.slice(at, at + 2), 16));
  }
  return [...colour.matchAll(/\d+/g)].map((match) => Number(match[0]));
}

/** Perceived lightness, near enough for "is this darker than that": WCAG relative luminance. */
function luminance(colour: string): number {
  const [r, g, b] = channels(colour).map((channel) => {
    const c = channel / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  }) as [number, number, number];
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

/** WCAG contrast between two colours written as `rgb(…)`. */
function contrast(a: string, b: string): number {
  const [lighter, darker] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [number, number];
  return (lighter + 0.05) / (darker + 0.05);
}

describe("picking a colour for a background", () => {
  it("writes white on a dark accent and near-black on a light one", () => {
    expect(readableTextOn("#171717"), "a near-black accent takes white text").toBe("#ffffff");
    expect(readableTextOn("#f4e4b8"), "a pale yellow takes dark text").toBe("#0a0a0a");
  });

  it("picks the one that actually reads better, not the one that matches the theme", () => {
    for (const accent of ["#4f9cf0", "#7b61ff", "#c47a50", "#d0d0d6", "#101820"]) {
      const chosen = readableTextOn(accent);
      const other = chosen === "#ffffff" ? "#0a0a0a" : "#ffffff";
      expect(contrast(accent, chosen), `${chosen} beats ${other} on ${accent}`).toBeGreaterThanOrEqual(
        contrast(accent, other),
      );
    }
  });
});

describe("deriving a window from five colours", () => {
  it("moves the panels toward the text, whichever way round the two are", () => {
    const light = deriveCustomColors(DEFAULT_CUSTOM_COLORS, "light");
    const dark = deriveCustomColors(DEFAULT_CUSTOM_COLORS_DARK, "dark");

    // The rule is the same in both directions and lands on the same side: a panel is a step away
    // from the page, toward the colour of the text on it — up in a light theme, down in a dark one.
    expect(luminance(light["--card"] as string), "a light theme's panel is darker than its page").toBeLessThan(
      luminance(light["--background"] as string),
    );
    expect(luminance(dark["--card"] as string), "a dark theme's panel is lighter than its page").toBeGreaterThan(
      luminance(dark["--background"] as string),
    );
  });

  it("keeps the muted text legible on the page it sits on", () => {
    for (const [set, theme] of [
      [DEFAULT_CUSTOM_COLORS, "light"],
      [DEFAULT_CUSTOM_COLORS_DARK, "dark"],
    ] as const) {
      const derived = deriveCustomColors(set, theme);
      expect(
        contrast(derived["--muted-foreground"] as string, derived["--background"] as string),
        `${theme}: quiet text is still text`,
      ).toBeGreaterThan(3);
      expect(
        contrast(derived["--foreground"] as string, derived["--background"] as string),
        `${theme}: and the loud text is louder`,
      ).toBeGreaterThan(contrast(derived["--muted-foreground"] as string, derived["--background"] as string));
    }
  });

  it("never writes the accent's own text in a colour that cannot be read on it", () => {
    for (const primary of ["#ffffff", "#171717", "#4f9cf0", "#f4e4b8", "#7b61ff"]) {
      const derived = deriveCustomColors({ ...DEFAULT_CUSTOM_COLORS, primary }, "light");
      expect(
        contrast(derived["--primary-foreground"] as string, derived["--primary"] as string),
        `${primary} carries readable text`,
      ).toBeGreaterThan(3);
    }
  });

  it("dresses the pane, in the half it was asked for", () => {
    const light = deriveCustomColors(DEFAULT_CUSTOM_COLORS, "light");
    const dark = deriveCustomColors(DEFAULT_CUSTOM_COLORS_DARK, "dark");

    // Both halves exist at once and are named apart: the terminal is drawn in whichever it was told
    // to, which is not always the theme of the window around it.
    expect(Object.keys(light)).toContain("--terminal-light-background");
    expect(Object.keys(light)).not.toContain("--terminal-dark-background");
    expect(light["--terminal-light-background"]).toBe(light["--card"]);
    expect(dark["--terminal-dark-background"]).toBe(dark["--card"]);
  });

  it("keeps the page's own colour exactly as it was picked", () => {
    const derived = deriveCustomColors({ ...DEFAULT_CUSTOM_COLORS, background: "#101820" }, "light");

    expect(derived["--background"], "what somebody chose is not nudged into place").toBe("rgb(16 24 32)");
  });
});

describe("mixing two colours", () => {
  it("returns the ends at the ends and the middle in the middle", () => {
    expect(mix("#000000", "#ffffff", 0)).toBe("rgb(0 0 0)");
    expect(mix("#000000", "#ffffff", 1)).toBe("rgb(255 255 255)");
    expect(mix("#000000", "#ffffff", 0.5)).toBe("rgb(128 128 128)");
  });
});
