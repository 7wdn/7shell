/**
 * The two terminal palettes have to define the same colours.
 *
 * xterm is told a colour per name and falls back to its own default when one is missing, which on
 * a light background shows as black text on black — a rendering bug that is really a missing token.
 * The list below is the palette the pane reads; this test fails when the stylesheet stops
 * providing one of them, in either palette.
 */
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { describe, expect, it } from "vitest";

const STYLESHEET = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..", "..", "styles", "globals.css");

/** Every custom property the terminal reads, and both palettes must define. */
const REQUIRED = [
  "--terminal-background",
  "--terminal-foreground",
  "--terminal-cursor",
  "--terminal-selection",
  "--terminal-black",
  "--terminal-red",
  "--terminal-green",
  "--terminal-yellow",
  "--terminal-blue",
  "--terminal-magenta",
  "--terminal-cyan",
  "--terminal-white",
  "--terminal-bright-black",
  "--terminal-bright-red",
  "--terminal-bright-green",
  "--terminal-bright-yellow",
  "--terminal-bright-blue",
  "--terminal-bright-magenta",
  "--terminal-bright-cyan",
  "--terminal-bright-white",
];

function paletteBlock(stylesheet: string, className: string): string {
  const start = stylesheet.indexOf(`.${className} {`);
  expect(start, `${className} is missing from the stylesheet`).toBeGreaterThan(-1);
  const end = stylesheet.indexOf("}", start);
  return stylesheet.slice(start, end);
}

describe("terminal palettes", () => {
  const stylesheet = readFileSync(STYLESHEET, "utf8");

  for (const className of ["terminal-dark", "terminal-light"]) {
    it(`${className} defines every colour the terminal reads`, () => {
      const block = paletteBlock(stylesheet, className);
      const missing = REQUIRED.filter((name) => !block.includes(`${name}:`));

      expect(missing, `${className} does not define ${missing.join(", ")}`).toEqual([]);
    });
  }

  it("gives the two palettes different backgrounds", () => {
    const dark = paletteBlock(stylesheet, "terminal-dark");
    const light = paletteBlock(stylesheet, "terminal-light");

    expect(dark).not.toBe(light);
    expect(dark).toMatch(/--terminal-background: oklch\(0\.[12]/);
    expect(light).toMatch(/--terminal-background: oklch\(0\.9/);
  });
});
