import { describe, expect, it } from "vitest";

import { completionSiteAt } from "@/lib/commands/tokenize";

/** The caret at the end of `line`, which is where somebody typing is. */
function at(line: string): ReturnType<typeof completionSiteAt> {
  return completionSiteAt(line, line.length);
}

describe("where the caret is", () => {
  it("finds the word and where it starts, so a candidate replaces only that word", () => {
    expect(at("ls -l /va")).toMatchObject({ start: 6, prefix: "/va", commandPosition: false });
  });

  it("reads a word behind a wrapper as the command word", () => {
    expect(at("sudo systemc")).toMatchObject({ prefix: "systemc", commandPosition: true, command: null });
  });

  it("stops calling it a command word once the wrapper has arguments of its own", () => {
    // `sudo -u root ls`: there is no way to tell from one line which word was meant to be the
    // command, so nothing is claimed and no command candidates are offered.
    expect(at("sudo -u root ls")).toMatchObject({ commandPosition: false });
  });

  it("knows the command an argument belongs to", () => {
    expect(at("cat a | grep e")).toMatchObject({ start: 13, prefix: "e", command: "grep" });
  });

  it("reads the word after a separator as a command word again", () => {
    expect(at("ls | ")).toMatchObject({ prefix: "", commandPosition: true });
    expect(at("systemctl restart nginx && journalc")).toMatchObject({
      prefix: "journalc",
      commandPosition: true,
      command: null,
    });
  });

  it("hands back the text inside quotes and says the word is quoted", () => {
    expect(at('cd "/va')).toMatchObject({ start: 4, prefix: "/va", quoted: true });
  });

  it("says nothing is offered in a comment", () => {
    expect(at("ls # note")).toMatchObject({ comment: true });
  });

  it("does not read a hash inside quotes as a comment", () => {
    expect(at('echo "#"')).toMatchObject({ comment: false });
  });

  it("has nothing to complete on an empty line, and calls it the command word", () => {
    expect(completionSiteAt("", 0)).toMatchObject({ start: 0, prefix: "", commandPosition: true, command: null });
  });

  it("counts the words after the command, wrappers not among them", () => {
    expect(at("systemc")).toMatchObject({ index: 0, commandPosition: true });
    expect(at("systemctl ")).toMatchObject({ index: 1, command: "systemctl", prefix: "" });
    expect(at("systemctl restart nginx")).toMatchObject({ index: 2, command: "systemctl" });
    expect(at("sudo systemctl ")).toMatchObject({ index: 1, command: "systemctl" });
    expect(at("ls | git status")).toMatchObject({ index: 1, command: "git" });
  });
});
