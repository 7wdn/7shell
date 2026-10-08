import { describe, expect, it } from "vitest";

import { COMMAND_CATALOG, descriptionKeyFor } from "@/lib/commands/catalog";

import enCommands from "@/i18n/locales/en/commands.json";
import zhCommands from "@/i18n/locales/zh-CN/commands.json";

function described(locale: { commands: { completion: { command: Record<string, string> } } }): string[] {
  return Object.keys(locale.commands.completion.command);
}

describe("the catalogue of commands", () => {
  it("names each command once", () => {
    const names = COMMAND_CATALOG.map((entry) => entry.name);
    expect(new Set(names).size).toBe(names.length);
  });

  it("only has one-line descriptions for commands it actually offers", () => {
    const names = new Set(COMMAND_CATALOG.map((entry) => entry.name));
    for (const locale of [enCommands, zhCommands]) {
      for (const name of described(locale)) {
        expect(names.has(name), `${name} has a description but is not in the catalogue`).toBe(true);
      }
    }
  });

  it("describes the commands a person reaches for first, in both languages", () => {
    const both = [described(enCommands), described(zhCommands)];
    expect(both[0]?.length).toBeGreaterThanOrEqual(40);
    expect(both[0]?.slice().sort()).toEqual(both[1]?.slice().sort());

    for (const name of ["ls", "grep", "tar", "systemctl", "docker", "git"]) {
      expect(enCommands.commands.completion.command, `${name} is described in English`).toHaveProperty(
        name,
        expect.any(String),
      );
      expect(zhCommands.commands.completion.command, `${name} is described in Chinese`).toHaveProperty(
        name,
        expect.any(String),
      );
    }
  });

  it("reads a description where the locale files keep it", () => {
    expect(descriptionKeyFor("grep")).toBe("commands.completion.command.grep");
  });
});
