import { afterEach, describe, expect, it, vi } from "vitest";

import { TerminalInputOrigin } from "@/lib/terminal/input-origin";

afterEach(() => vi.useRealTimers());

describe("broadcast input origins", () => {
  it("recognizes a keyboard control sequence while leaving terminal replies local", () => {
    const origin = new TerminalInputOrigin();
    origin.key("\x1b[A");
    expect(origin.take("\x1b[12;4R")).toBe(false);
    expect(origin.take("\x1b[A")).toBe(true);
    expect(origin.take("\x1b[A")).toBe(false);
    origin.dispose();
  });

  it("recognizes plain and bracketed pastes with normalized newlines", () => {
    const origin = new TerminalInputOrigin();
    origin.paste("echo one\necho two");
    expect(origin.take("\x1b[200~echo one\recho two\x1b[201~")).toBe(true);
    origin.paste("echo three\r\n");
    expect(origin.take("echo three\r")).toBe(true);
    origin.dispose();
  });

  it("keeps the composition commit through xterm's deferred read but refuses protocol replies", async () => {
    vi.useFakeTimers();
    const origin = new TerminalInputOrigin();
    origin.text();
    expect(origin.take("\x1b[0n")).toBe(false);
    await vi.advanceTimersToNextTimerAsync();
    expect(origin.take("中文")).toBe(true);
    origin.text();
    await vi.runAllTimersAsync();
    expect(origin.take("unrelated")).toBe(false);
    origin.dispose();
  });

  it("expires an unused keyboard marker before a later protocol event can claim it", async () => {
    const origin = new TerminalInputOrigin();
    origin.key("a");
    await Promise.resolve();
    expect(origin.take("a")).toBe(false);
    origin.dispose();
  });
});
