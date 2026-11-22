/**
 * The contrast arithmetic the terminal preview leans on.
 *
 * What matters here is that the answer agrees with the definition a renderer works from: the ratio
 * is the WCAG one, the direction is away from the background's own brightness, and a pair that
 * already clears the bar is left exactly as it was written — a preview that rewrote healthy
 * colours would be lying about the setting.
 */
import { describe, expect, it } from "vitest";

import { contrastRatio, ensureMinimumContrast, parsePreviewColour } from "@/lib/terminal/contrast";

describe("reading the colours a preview meets", () => {
  it("reads hex in the short and long form", () => {
    expect(parsePreviewColour("#abc")).toEqual({ r: 170, g: 187, b: 204 });
    expect(parsePreviewColour("#AaBbCc")).toEqual({ r: 170, g: 187, b: 204 });
  });

  it("reads rgb(), and the oklch the palette tokens are declared in", () => {
    expect(parsePreviewColour("rgb(1, 2, 3)")).toEqual({ r: 1, g: 2, b: 3 });
    // oklch(0.68 0.16 22) is the stylesheet's terminal red: a muted red, not a grey.
    const red = parsePreviewColour("oklch(0.68 0.16 22)");
    expect(red).not.toBeNull();
    expect(red!.r).toBeGreaterThan(red!.b!);
  });

  it("refuses what is not a colour rather than guessing", () => {
    expect(parsePreviewColour("var(--terminal-red)")).toBeNull();
    expect(parsePreviewColour("")).toBeNull();
  });
});

describe("the minimum-contrast answer", () => {
  it("leaves a pair alone that already clears the bar", () => {
    expect(ensureMinimumContrast("#ffffff", "#000000", 8)).toBe("#ffffff");
    expect(ensureMinimumContrast("#ff0000", "#000000", 4)).toBe("#ff0000");
  });

  it("leaves colours alone when the setting is off", () => {
    expect(ensureMinimumContrast("#1a1a1a", "#000000", 1)).toBe("#1a1a1a");
  });

  it("lightens text on a dark background until the bar is met", () => {
    const raised = ensureMinimumContrast("#301010", "#101010", 7);
    expect(raised).not.toBe("#301010");
    expect(contrastRatio(raised, "#101010")).toBeGreaterThanOrEqual(7);
    // Lighter, not darker: the blend moved towards white.
    expect(contrastRatio(raised, "#101010")).toBeGreaterThan(contrastRatio("#301010", "#101010")!);
  });

  it("darkens text on a light background", () => {
    const lowered = ensureMinimumContrast("#e8d0d0", "#f7f7f7", 7);
    expect(contrastRatio(lowered, "#f7f7f7")).toBeGreaterThanOrEqual(7);
    expect(parsePreviewColour(lowered)!.r).toBeLessThan(232);
  });

  it("answers unchanged for a colour it cannot read", () => {
    expect(ensureMinimumContrast("var(--terminal-red)", "#000000", 8)).toBe("var(--terminal-red)");
  });

  it("computes the ratio the definition gives", () => {
    expect(contrastRatio("#ffffff", "#000000")).toBeCloseTo(21, 0);
    expect(contrastRatio("#000000", "#000000")).toBeCloseTo(1, 5);
  });
});
