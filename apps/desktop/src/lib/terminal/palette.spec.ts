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

function block(stylesheet: string, selector: string): string {
  const start = stylesheet.indexOf(`${selector} {`);
  expect(start, `${selector} is missing from the stylesheet`).toBeGreaterThan(-1);
  return stylesheet.slice(start, stylesheet.indexOf("\n}", start));
}

describe("terminal palettes", () => {
  const stylesheet = readFileSync(STYLESHEET, "utf8");

  for (const className of ["terminal-dark", "terminal-light"]) {
    it(`.${className} defines every colour the terminal reads`, () => {
      const body = block(stylesheet, `.${className}`);
      const missing = REQUIRED.filter((name) => !body.includes(`${name}:`));

      expect(missing, `.${className} does not define ${missing.join(", ")}`).toEqual([]);
    });
  }

  /**
   * The two panes read their own half of the colour theme, and both halves are written down.
   *
   * There is no colour in either class, by design: the pane's four surfaces come from the palette —
   * for the pane's own theme rather than the window's, so a dark pane is dark beside a light window
   * — and the palette that draws no block of its own puts them in `:root`. A half that was never
   * declared would leave the pane with a value that is not a colour, which is what these two cases
   * together are for.
   */
  it("reads its own half of the colour theme in each pane", () => {
    expect(block(stylesheet, ".terminal-dark"), "the dark pane reads the dark half").toContain(
      "var(--terminal-dark-background)",
    );
    expect(block(stylesheet, ".terminal-light"), "the light pane reads the light half").toContain(
      "var(--terminal-light-background)",
    );
  });

  it("finds both halves declared, for the palette that has no block of its own", () => {
    const root = block(stylesheet, ":root");
    for (const theme of ["dark", "light"]) {
      for (const part of ["background", "foreground", "cursor", "selection"]) {
        expect(root, `the base palette dresses the ${theme} pane too`).toContain(`--terminal-${theme}-${part}:`);
      }
    }
  });
});
