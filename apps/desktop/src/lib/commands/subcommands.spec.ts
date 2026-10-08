import { describe, expect, it } from "vitest";

import { subcommandKeyFor, subcommandsOf } from "@/lib/commands/subcommands";

import enCommands from "@/i18n/locales/en/commands.json";
import zhCommands from "@/i18n/locales/zh-CN/commands.json";

type Descriptions = Record<string, Record<string, string>>;

function described(locale: { commands: { completion: { subcommand: Descriptions } } }): Descriptions {
  return locale.commands.completion.subcommand;
}

describe("the words after a command", () => {
  it("answers nothing for a command the table does not know", () => {
    expect(subcommandsOf("curl")).not.toHaveLength(0);
    expect(subcommandsOf("definitely-not-a-command")).toEqual([]);
  });

  it("answers the same for a command whatever case it was typed in", () => {
    expect(subcommandsOf("Git")).toEqual(subcommandsOf("git"));
  });

  it("names each word once inside a command", () => {
    for (const command of ["git", "systemctl", "docker", "kubectl", "apt", "grep", "ls", "curl"]) {
      const words = subcommandsOf(command);
      expect(new Set(words).size, command).toBe(words.length);
    }
  });

  it("only has one-line descriptions for words it actually offers, in both languages", () => {
    const both = [described(enCommands), described(zhCommands)];
    expect(Object.keys(both[0] ?? {}).sort()).toEqual(Object.keys(both[1] ?? {}).sort());

    for (const table of both) {
      for (const [command, words] of Object.entries(table)) {
        expect(subcommandsOf(command), `${command} is described but not in the table`).not.toHaveLength(0);
        for (const word of Object.keys(words)) {
          expect(subcommandsOf(command), `${command} ${word} is described but not offered`).toContain(word);
        }
      }
    }
  });

  it("describes the words somebody reaches for first, in both languages", () => {
    for (const [command, word] of [
      ["git", "status"],
      ["systemctl", "restart"],
      ["docker", "logs"],
    ] as const) {
      expect(enCommands.commands.completion.subcommand, `${command} ${word} in English`).toHaveProperty(
        [command, word],
        expect.any(String),
      );
      expect(zhCommands.commands.completion.subcommand, `${command} ${word} in Chinese`).toHaveProperty(
        [command, word],
        expect.any(String),
      );
    }
  });

  it("reads a description where the locale files keep it", () => {
    expect(subcommandKeyFor("Git", "-l")).toBe("commands.completion.subcommand.git.-l");
  });
});
