import { describe, expect, it } from "vitest";

import {
  schemeAnsi,
  schemeCssVars,
  schemeTone,
  TERMINAL_COLOR_SCHEMES,
  terminalSchemeById,
  withAlpha,
} from "@/lib/terminal/schemes";

const HEX = /^#[0-9a-f]{6}$/i;

describe("the terminal colour schemes", () => {
  it("ships a catalogue the picker can actually offer", () => {
    expect(TERMINAL_COLOR_SCHEMES.length).toBeGreaterThan(150);
    expect(new Set(TERMINAL_COLOR_SCHEMES.map((scheme) => scheme.id)).size).toBe(TERMINAL_COLOR_SCHEMES.length);
  });

  it("gives every scheme all sixteen colours and four valid surfaces", () => {
    const broken = TERMINAL_COLOR_SCHEMES.filter(
      (scheme) =>
        scheme.colors.length !== 16 ||
        !HEX.test(scheme.foreground) ||
        !HEX.test(scheme.background) ||
        !HEX.test(scheme.cursor) ||
        scheme.colors.some((colour) => !HEX.test(colour)),
    );
    expect(broken.map((scheme) => scheme.id)).toEqual([]);
  });

  it("finds a scheme by id, and nothing for an id it does not ship", () => {
    expect(terminalSchemeById("dracula")?.background).toBe("#1e1f29");
    expect(terminalSchemeById("no-such-scheme")).toBeUndefined();
    expect(terminalSchemeById(null)).toBeUndefined();
    expect(terminalSchemeById("")).toBeUndefined();
  });

  it("groups schemes by the background they were drawn on", () => {
    expect(schemeTone(terminalSchemeById("dracula")!)).toBe("dark");
    expect(schemeTone(terminalSchemeById("solarized-light")!)).toBe("light");
  });

  it("hands the colours to xterm and to the preview under the same names", () => {
    const dracula = terminalSchemeById("dracula")!;
    const ansi = schemeAnsi(dracula);
    expect(ansi.black).toBe("#000000");
    expect(ansi.brightWhite).toBe("#ffffff");

    const vars = schemeCssVars(dracula);
    expect(vars["--terminal-black"]).toBe("#000000");
    expect(vars["--terminal-bright-white"]).toBe("#ffffff");
  });

  it("adds an alpha in a form both xterm and the browser read", () => {
    expect(withAlpha("#f8f8f2", 0.3)).toBe("#f8f8f24d");
    expect(withAlpha("#f8f8f2", 0)).toBe("#f8f8f200");
    expect(withAlpha("#f8f8f2", 1)).toBe("#f8f8f2ff");
  });
});
