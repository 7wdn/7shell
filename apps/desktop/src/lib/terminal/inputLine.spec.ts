import { describe, expect, it } from "vitest";

import { InputLine } from "./inputLine";

describe("InputLine", () => {
  it("follows printable characters and backspace", () => {
    const line = new InputLine();
    expect(line.feed("git sta")).toMatchObject({ line: "git sta", trusted: true, atEnd: true });
    expect(line.feed("\x7f")).toMatchObject({ line: "git st", trusted: true });
    expect(line.feed("atus")).toMatchObject({ line: "git status" });
    expect(line.feed("\x7f\x7f")).toMatchObject({ line: "git stat" });
  });

  it("removes a whole code point on backspace", () => {
    const line = new InputLine();
    line.feed("echo 你好");
    line.feed("\x7f");
    expect(line.snapshot().line).toBe("echo 你");
  });

  it("clears the line on Ctrl-U and Ctrl-C, and trusts the empty line again", () => {
    for (const kill of ["\x15", "\x03"]) {
      const line = new InputLine();
      line.feed("git checkout");
      const state = line.feed(kill);
      expect(state).toMatchObject({ line: "", trusted: true, atEnd: true });
    }
  });

  it("deletes the last word on Ctrl-W", () => {
    const line = new InputLine();
    line.feed("git checkout --quiet ");
    line.feed("\x17");
    expect(line.snapshot().line).toBe("git checkout ");
  });

  it("drops the line at Enter and says it was executed", () => {
    const line = new InputLine();
    line.feed("uptime");
    expect(line.feed("\r")).toMatchObject({ line: "", trusted: true, executed: true });
  });

  it("gives up on the line at an escape sequence, and stays given up until Enter", () => {
    const line = new InputLine();
    line.feed("git ");
    // Up arrow: history recall. The shell now holds a line this side did not type.
    const state = line.feed("\x1b[A");
    expect(state.trusted).toBe(false);
    expect(state.atEnd).toBe(false);
    expect(line.feed("ch").trusted).toBe(false);
    expect(line.feed("\r").trusted).toBe(true);
  });

  it("gives up on the line at Tab and at a bracketed paste", () => {
    const line = new InputLine();
    line.feed("git ");
    expect(line.feed("\t").trusted).toBe(false);

    const pasted = new InputLine();
    pasted.feed("echo ");
    expect(pasted.feed("\x1b[200~rm -rf /tmp/x\x1b[201~").trusted).toBe(false);
    expect(pasted.snapshot().line).toBe("echo ");
  });

  it("takes a completion's line as known exactly", () => {
    const line = new InputLine();
    line.feed("\x1b[A");
    line.setLine("git status");
    expect(line.snapshot()).toMatchObject({ line: "git status", trusted: true, atEnd: true });
  });
});
