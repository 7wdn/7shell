/**
 * What a font setting is worth, and what happens to a name somebody typed.
 *
 * The contract here is the one thing about a font choice that is not visible: what is stored is a
 * stack, and the fallbacks after the chosen family are why a name that is not installed leaves a
 * readable window rather than whatever the browser draws with when it matches nothing.
 */
import { describe, expect, it } from "vitest";

import {
  buildFontOptions,
  cssFontFamilyForName,
  fontOptionsFor,
  normalizeCustomFontFamilyInput,
  readableFontFamily,
} from "@/lib/settings/fonts";
import { DEFAULT_TERMINAL_FONT, DEFAULT_UI_FONT, FONT_PRESETS, SYSTEM_UI_FONT } from "@/types/app";

describe("a family name inside a stack", () => {
  it("is quoted, because a name with a space in it is two families without the quotes", () => {
    expect(cssFontFamilyForName("Fira Code")).toBe("'Fira Code', monospace");
  });

  it("has its own quote escaped rather than ending the name", () => {
    // A font called `Bob's Mono` would otherwise close the string and make the rest of the stack
    // part of the name — which the browser then fails to match, silently.
    expect(cssFontFamilyForName("Bob's Mono")).toBe("'Bob\\'s Mono', monospace");
  });

  it("reads back out as the name a person would say", () => {
    expect(readableFontFamily("'Bob\\'s Mono', monospace")).toBe("Bob's Mono");
    expect(readableFontFamily('"JetBrains Mono", monospace')).toBe("JetBrains Mono");
    expect(readableFontFamily("monospace")).toBe("monospace");
  });
});

describe("a name somebody typed instead of picking", () => {
  it("is taken as a whole stack when it looks like one", () => {
    expect(normalizeCustomFontFamilyInput("Inter, sans-serif")).toBe("Inter, sans-serif");
    expect(normalizeCustomFontFamilyInput("'My Font', serif")).toBe("'My Font', serif");
  });

  it("is quoted and given a fallback when it is just a name", () => {
    expect(normalizeCustomFontFamilyInput("My Font")).toBe("'My Font', monospace");
  });

  it("is nothing at all when there is nothing to take", () => {
    expect(normalizeCustomFontFamilyInput("   ")).toBe("");
  });
});

describe("the list of families", () => {
  it("puts what this build names first, then the machine's own", () => {
    const options = buildFontOptions(["Arial"], [], ["First"]);

    expect(options[0]).toBe("First");
    expect(options.slice(1, 8), "the fixed-width families people arrive with").toEqual(
      FONT_PRESETS.map((preset) => preset.value),
    );
    expect(options[8]).toBe("'Arial', monospace");
  });

  it("carries the family in force, so a choice made elsewhere is still shown as the choice", () => {
    const options = buildFontOptions([], ["'Not Installed Here', monospace"], []);
    expect(options.at(-1)).toBe("'Not Installed Here', monospace");
  });

  it("says a stack once, which is the only way the same family can arrive twice", () => {
    const options = buildFontOptions([], ["'Consolas', 'Courier New', monospace"], []);
    expect(options.filter((stack) => stack === "'Consolas', 'Courier New', monospace").length).toBe(1);
  });

  it("offers a family the machine also has, once from each side", () => {
    // Consolas is a preset and it is installed, and the two entries are different stacks: one carries
    // the fallbacks this build gives it, the other is the machine's own spelling. Both are offered,
    // preset first — which is the row that still works when the family is not there.
    const options = buildFontOptions(["Consolas"], ["'Consolas', monospace"], []);

    expect(options.filter((stack) => readableFontFamily(stack) === "Consolas")).toEqual([
      "'Consolas', 'Courier New', monospace",
      "'Consolas', monospace",
    ]);
  });
});

describe("what each of the two settings offers", () => {
  it("leads the interface with the system's own face and this build's default", () => {
    const options = fontOptionsFor("interface", ["Arial"], DEFAULT_UI_FONT);

    expect(options.slice(0, 2)).toEqual([SYSTEM_UI_FONT, DEFAULT_UI_FONT]);
    expect(options.slice(2, 9), "and the same fixed-width faces under them").toEqual(
      FONT_PRESETS.map((preset) => preset.value),
    );
    // The family in force is one of the two named entries here, so the list does not repeat it at the
    // end — it is only carried there when nothing above it has named it.
    expect(options.at(-1)).toBe("'Arial', monospace");
  });

  it("leads the terminal with the family this build ships, and the same faces under it", () => {
    const options = fontOptionsFor("terminal", ["Arial"], DEFAULT_TERMINAL_FONT);

    expect(options[0]).toBe(DEFAULT_TERMINAL_FONT);
    expect(options.slice(1, 8)).toEqual(FONT_PRESETS.map((preset) => preset.value));
    expect(options).toContain("monospace");
  });
});
