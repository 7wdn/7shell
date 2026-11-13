import { describe, expect, it } from "vitest";

import {
  importedSchemeId,
  mergeImportedSchemes,
  normalizeSchemeColour,
  parseTerminalSchemeImport,
} from "@/lib/terminal/importScheme";

const PALETTE = Array.from({ length: 16 }, (_, index) => `#${(index + 1).toString(16).padStart(2, "0")}1122`);

function xresources(name = "Mine"): string {
  return [
    `*.name: ${name}`,
    "*.foreground: #ffffff",
    "*.background: #101010",
    "*.cursorColor: #abcdef",
    ...PALETTE.map((colour, index) => `*.color${index}: ${colour}`),
  ].join("\n");
}

function windowsTerminal(): string {
  const keys = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightMagenta",
    "brightCyan",
    "brightWhite",
  ];
  return JSON.stringify({
    schemes: [
      {
        name: "Terminal test",
        foreground: "#f2f2f2",
        background: "#0c0c0c",
        cursorColor: "#f2f2f2",
        ...Object.fromEntries(keys.map((key, index) => [key, PALETTE[index]])),
      },
    ],
  });
}

function iterm(): string {
  const block = (key: string, colour: string): string => {
    const [r, g, b] = [1, 3, 5].map((offset) => Number.parseInt(colour.slice(offset, offset + 2), 16) / 255);
    return `<key>${key}</key><dict>
      <key>Red Component</key><real>${r}</real>
      <key>Green Component</key><real>${g}</real>
      <key>Blue Component</key><real>${b}</real>
    </dict>`;
  };
  return `<?xml version="1.0" encoding="UTF-8"?><plist><dict>
    ${block("Foreground Color", "#ffffff")}
    ${block("Background Color", "#101010")}
    ${block("Cursor Color", "#abcdef")}
    ${PALETTE.map((colour, index) => block(`Ansi ${index} Color`, colour)).join("")}
  </dict></plist>`;
}

describe("reading a colour scheme", () => {
  it("accepts the short and long hex forms, and refuses everything else", () => {
    expect(normalizeSchemeColour("#ABC")).toBe("#aabbcc");
    expect(normalizeSchemeColour("#AaBbCc")).toBe("#aabbcc");
    expect(normalizeSchemeColour("rgb(1,2,3)")).toBeNull();
    expect(normalizeSchemeColour("red")).toBeNull();
  });

  it("reads an Xresources block", () => {
    const result = parseTerminalSchemeImport(xresources());
    expect(result.format).toBe("xresources");
    expect(result.schemes).toHaveLength(1);
    expect(result.schemes[0]).toMatchObject({
      name: "Mine",
      foreground: "#ffffff",
      background: "#101010",
      cursor: "#abcdef",
    });
    expect(result.schemes[0]!.colors).toHaveLength(16);
  });

  it("reads a Windows Terminal JSON scheme", () => {
    const result = parseTerminalSchemeImport(windowsTerminal());
    expect(result.format).toBe("json");
    expect(result.schemes[0]?.name).toBe("Terminal test");
    expect(result.schemes[0]?.colors[1]).toBe(PALETTE[1]);
  });

  it("reads an iTerm2 plist", () => {
    const result = parseTerminalSchemeImport(iterm());
    expect(result.format).toBe("iterm2");
    expect(result.schemes[0]?.foreground).toBe("#ffffff");
    expect(result.schemes[0]?.background).toBe("#101010");
    expect(result.schemes[0]?.colors[0]).toBe(PALETTE[0]);
  });

  it("refuses text that is not a complete scheme, rather than padding it", () => {
    expect(parseTerminalSchemeImport("hello").schemes).toEqual([]);
    expect(parseTerminalSchemeImport(xresources().replace("*.color15: #101122", "")).schemes).toEqual([]);
    expect(parseTerminalSchemeImport("").format).toBe("unknown");
  });

  it("gives an imported scheme an id no name already owns", () => {
    const taken = new Set(["mine", "mine-2"]);
    expect(importedSchemeId("Mine", taken)).toBe("mine-3");
    expect(importedSchemeId("  ", new Set())).toBe("scheme");
  });

  it("counts the limit against imported schemes, not the shipped catalogue", () => {
    const parsed = parseTerminalSchemeImport(xresources()).schemes;
    // The catalogue is already larger than the limit; a merge that counted its ids would import
    // nothing at all, which is the failure this guards.
    const merged = mergeImportedSchemes(parsed, new Set(["3024-day", "dracula"]), 0, 60);
    expect(merged).toHaveLength(1);
    expect(merged[0]?.id).toBe("mine");

    expect(mergeImportedSchemes(parsed, new Set(), 60, 60)).toEqual([]);
    expect(mergeImportedSchemes([...parsed, ...parsed], new Set(), 59, 60)).toHaveLength(1);
  });
});
