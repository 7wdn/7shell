/**
 * Reading a terminal's tail, which is arithmetic over rows rather than anything about xterm.
 *
 * The two cases worth holding still are the ones the panel would get wrong silently: the blank rows
 * under the cursor, which are most of a buffer, and a snapshot that spans the top of the scrollback.
 */
import { describe, expect, it } from "vitest";

import { registerTerminalBuffer, tailOfBuffer, terminalTail } from "@/lib/terminal/buffers";

/** A buffer of rows, the shape a terminal's own scrollback has: text, and blank rows under it. */
function bufferOf(rows: readonly string[]): {
  readonly length: number;
  getLine(index: number): { translateToString(trimRight?: boolean): string } | undefined;
} {
  return {
    length: rows.length,
    getLine(index: number) {
      const row = rows[index];
      return row === undefined
        ? undefined
        : { translateToString: (trimRight?: boolean) => (trimRight ? row.trimEnd() : row) };
    },
  };
}

describe("the tail of a terminal buffer", () => {
  it("reads the rows the terminal has printed, not the blank ones under the cursor", () => {
    const buffer = bufferOf(["one", "two", "three", "", "", ""]);
    expect(tailOfBuffer(buffer, 10)).toBe("one\ntwo\nthree");
  });

  it("takes the last rows when there are more than were asked for", () => {
    const buffer = bufferOf(["one", "two", "three", "four"]);
    expect(tailOfBuffer(buffer, 2)).toBe("three\nfour");
  });

  it("reads everything when the scrollback is shorter than the request", () => {
    expect(tailOfBuffer(bufferOf(["one", "two"]), 80)).toBe("one\ntwo");
  });

  it("answers nothing for a terminal that has printed nothing", () => {
    expect(tailOfBuffer(bufferOf(["", ""]), 80)).toBe("");
    expect(tailOfBuffer(bufferOf([]), 80)).toBe("");
  });

  it("answers nothing when no lines were asked for", () => {
    expect(tailOfBuffer(bufferOf(["one"]), 0)).toBe("");
  });

  it("keeps a blank row between two lines of output", () => {
    expect(tailOfBuffer(bufferOf(["first", "", "second"]), 10)).toBe("first\n\nsecond");
  });
});

describe("the registry the panel reads through", () => {
  it("answers by handle while a pane is drawing it", () => {
    const unregister = registerTerminalBuffer("handle-1", (lines) => `tail:${lines}`);
    expect(terminalTail("handle-1", 40)).toBe("tail:40");
    unregister();
  });

  it("answers nothing once the pane has gone, rather than answering with what it last saw", () => {
    const unregister = registerTerminalBuffer("handle-2", () => "text");
    unregister();
    expect(terminalTail("handle-2", 80)).toBe("");
  });

  it("lets a second pane claim a handle, and does not let the first one unclaim it", () => {
    const first = registerTerminalBuffer("handle-3", () => "first");
    const second = registerTerminalBuffer("handle-3", () => "second");
    first();
    expect(terminalTail("handle-3", 10)).toBe("second");
    second();
  });
});
