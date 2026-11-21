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

const ANSI_NAMES = [
  "Black",
  "Red",
  "Green",
  "Yellow",
  "Blue",
  "Magenta",
  "Cyan",
  "White",
  "BrightBlack",
  "BrightRed",
  "BrightGreen",
  "BrightYellow",
  "BrightBlue",
  "BrightMagenta",
  "BrightCyan",
  "BrightWhite",
] as const;

function kitty(): string {
  // color0 in kitty's wide documented form: #090903030000 folds to #090300.
  const wide0 = "#090903030000";
  return [
    "foreground            #a5a2a2",
    "background            #090300",
    "cursor                #a5a2a2",
    "selection_background  #4a4543",
    "selection_foreground  #090300",
    `color0  ${wide0}`,
    ...PALETTE.slice(1).map((colour, index) => `color${index + 1}  ${colour}`),
  ].join("\n");
}

function ghostty(): string {
  // Ghostty themes in the wild write the hex without the `#`; the site's export keeps it.
  return [
    "background = 090300",
    "foreground = a5a2a2",
    "cursor-color = a5a2a2",
    "selection-background = 4a4543",
    ...PALETTE.map((colour, index) => `palette = ${index}=${colour}`),
  ].join("\n");
}

function alacritty(): string {
  const named = (section: string, offset: number): string =>
    [
      `[colors.${section}]`,
      ...Object.entries({ black: 0, red: 1, green: 2, yellow: 3, blue: 4, magenta: 5, cyan: 6, white: 7 }).map(
        ([key, slot]) => `${key} = '${PALETTE[offset + slot]}'`,
      ),
    ].join("\n");
  return [
    "# Colors (test)",
    "",
    "[colors.primary]",
    "background = '#090300'",
    "foreground = '#a5a2a2'",
    "",
    "[font]",
    "size = 12",
    "",
    named("normal", 0),
    "",
    named("bright", 8),
    "",
    "[colors.cursor]",
    "cursor = '#a5a2a2'",
    "text = '#090300'",
    "",
    "[colors.selection]",
    "background = '#4a4543'",
    "text = '#a5a2a2'",
  ].join("\n");
}

function warp(): string {
  const colours = (offset: number): string =>
    Object.entries({ black: 0, red: 1, green: 2, yellow: 3, blue: 4, magenta: 5, cyan: 6, white: 7 })
      .map(([key, slot]) => `    ${key}: '${PALETTE[offset + slot]}'`)
      .join("\n");
  return [
    "name: Warp test",
    "accent: '#268bd2'",
    "cursor: '#a5a2a2'",
    "background: '#090300'",
    "foreground: '#a5a2a2'",
    "details: darker",
    "terminal_colors:",
    "  normal:",
    colours(0),
    "  bright:",
    colours(8),
    "background_image:",
    "  path: /tmp/never.png",
    "  opacity: 20",
  ].join("\n");
}

function wezterm(): string {
  return [
    "# Test wezterm",
    "[colors]",
    'foreground = "#a5a2a2"',
    'background = "#090300"',
    'cursor_bg = "#a5a2a2"',
    'selection_bg = "#4a4543"',
    "",
    `ansi = [${PALETTE.slice(0, 8).join(",")}]`,
    `brights = [${PALETTE.slice(8).join(",")}]`,
  ].join("\n");
}

function konsole(): string {
  const rgb = (colour: string): string =>
    [1, 3, 5].map((offset) => Number.parseInt(colour.slice(offset, offset + 2), 16)).join(",");
  const lines = [
    "[General]",
    "Description=Console test",
    "",
    "[Foreground]",
    `Color=${rgb("#a5a2a2")}`,
    "",
    "[Background]",
    `Color=${rgb("#090300")}`,
    "",
  ];
  PALETTE.slice(0, 8).forEach((colour, index) => lines.push(`[Color${index}]`, `Color=${rgb(colour)}`, ""));
  PALETTE.slice(8).forEach((colour, index) => lines.push(`[Color${index}Intense]`, `Color=${rgb(colour)}`, ""));
  // The faint sections repeat the normal colours; they must not override the intense ones.
  lines.push("[Color0Faint]", `Color=${rgb("#000000")}`);
  return lines.join("\n");
}

function vscodeWorkbench(): string {
  const customizations: Record<string, string> = {
    "terminal.foreground": "#a5a2a2",
    "terminal.background": "#090300",
    "terminalCursor.foreground": "#a5a2a2",
    "terminal.selectionBackground": "#4a4543",
  };
  ANSI_NAMES.forEach((name, index) => {
    customizations[`terminal.ansi${name}`] = PALETTE[index]!;
  });
  return JSON.stringify({ workbench: { colorCustomizations: customizations } }, null, 2);
}

function vscodeTheme(): string {
  const colors: Record<string, string> = {
    "terminal.background": "#090300",
    "terminal.foreground": "#a5a2a2",
  };
  ANSI_NAMES.forEach((name, index) => {
    colors[`terminal.ansi${name}`] = PALETTE[index]!;
  });
  return JSON.stringify({ name: "Theme test", type: "dark", colors }, null, 2);
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

  it("reads a kitty conf, folding the wide hex form", () => {
    const result = parseTerminalSchemeImport(kitty());
    expect(result.format).toBe("kitty");
    expect(result.schemes[0]?.foreground).toBe("#a5a2a2");
    expect(result.schemes[0]?.background).toBe("#090300");
    expect(result.schemes[0]?.cursor).toBe("#a5a2a2");
    expect(result.schemes[0]?.selectionBackground).toBe("#4a4543");
    expect(result.schemes[0]?.colors[0]).toBe("#090300");
    expect(result.schemes[0]?.colors[15]).toBe(PALETTE[15]);
  });

  it("reads a ghostty conf with the hash-less hex it ships", () => {
    const result = parseTerminalSchemeImport(ghostty());
    expect(result.format).toBe("ghostty");
    expect(result.schemes[0]?.background).toBe("#090300");
    expect(result.schemes[0]?.foreground).toBe("#a5a2a2");
    expect(result.schemes[0]?.cursor).toBe("#a5a2a2");
    expect(result.schemes[0]?.colors[9]).toBe(PALETTE[9]);
  });

  it("reads an alacritty TOML and leaves other tables alone", () => {
    const result = parseTerminalSchemeImport(alacritty());
    expect(result.format).toBe("alacritty");
    expect(result.schemes[0]?.background).toBe("#090300");
    expect(result.schemes[0]?.cursor).toBe("#a5a2a2");
    expect(result.schemes[0]?.selectionBackground).toBe("#4a4543");
    expect(result.schemes[0]?.colors[4]).toBe(PALETTE[4]);
    expect(result.schemes[0]?.colors[11]).toBe(PALETTE[11]);
  });

  it("reads a warp theme and stops at the terminal_colors block", () => {
    const result = parseTerminalSchemeImport(warp());
    expect(result.format).toBe("warp");
    expect(result.schemes[0]?.name).toBe("Warp test");
    expect(result.schemes[0]?.foreground).toBe("#a5a2a2");
    expect(result.schemes[0]?.cursor).toBe("#a5a2a2");
    expect(result.schemes[0]?.colors[7]).toBe(PALETTE[7]);
    expect(result.schemes[0]?.colors[8]).toBe(PALETTE[8]);
  });

  it("reads a wezterm toml with its ansi and brights arrays", () => {
    const result = parseTerminalSchemeImport(wezterm());
    expect(result.format).toBe("wezterm");
    expect(result.schemes[0]?.foreground).toBe("#a5a2a2");
    expect(result.schemes[0]?.cursor).toBe("#a5a2a2");
    expect(result.schemes[0]?.colors[0]).toBe(PALETTE[0]);
    expect(result.schemes[0]?.colors[12]).toBe(PALETTE[12]);
  });

  it("reads a konsole colorscheme with decimal colours and intense variants", () => {
    const result = parseTerminalSchemeImport(konsole());
    expect(result.format).toBe("konsole");
    expect(result.schemes[0]?.name).toBe("Console test");
    expect(result.schemes[0]?.foreground).toBe("#a5a2a2");
    expect(result.schemes[0]?.background).toBe("#090300");
    expect(result.schemes[0]?.colors[0]).toBe(PALETTE[0]);
    expect(result.schemes[0]?.colors[8]).toBe(PALETTE[8]);
    expect(result.schemes[0]?.colors).toHaveLength(16);
  });

  it("reads VS Code's prefixed customizations and its theme files", () => {
    const workbench = parseTerminalSchemeImport(vscodeWorkbench());
    expect(workbench.format).toBe("vscode");
    expect(workbench.schemes[0]?.foreground).toBe("#a5a2a2");
    expect(workbench.schemes[0]?.cursor).toBe("#a5a2a2");
    expect(workbench.schemes[0]?.selectionBackground).toBe("#4a4543");
    expect(workbench.schemes[0]?.colors[1]).toBe(PALETTE[1]);

    const theme = parseTerminalSchemeImport(vscodeTheme());
    expect(theme.format).toBe("vscode");
    expect(theme.schemes[0]?.name).toBe("Theme test");
    expect(theme.schemes[0]?.colors[15]).toBe(PALETTE[15]);
  });

  it("refuses dialect files that are missing colours, rather than padding them", () => {
    expect(parseTerminalSchemeImport(kitty().replace(/\ncolor15.*/, "")).schemes).toEqual([]);
    expect(parseTerminalSchemeImport(ghostty().replace(/\npalette = 15=.*/, "")).schemes).toEqual([]);
    expect(parseTerminalSchemeImport(alacritty().replace("[colors.bright]", "[colors.other]")).schemes).toEqual([]);
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
