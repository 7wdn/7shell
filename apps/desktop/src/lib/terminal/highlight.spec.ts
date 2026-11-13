import { describe, expect, it } from "vitest";

import { compileHighlightRules, highlightMatches } from "@/lib/terminal/highlight";
import type { TerminalHighlightRule } from "@/types/app";

function rule(pattern: string, overrides: Partial<TerminalHighlightRule> = {}): TerminalHighlightRule {
  return {
    id: pattern,
    pattern,
    color: "#f59e0b",
    isRegex: false,
    caseSensitive: false,
    ...overrides,
  };
}

describe("the terminal's highlight rules", () => {
  it("finds a plain word without treating it as a pattern", () => {
    const compiled = compileHighlightRules([rule("a.b")]);
    expect(highlightMatches("a.b axb a.b", compiled).map((match) => [match.start, match.end])).toEqual([
      [0, 3],
      [8, 11],
    ]);
  });

  it("reads a pattern as one when the rule says so", () => {
    const compiled = compileHighlightRules([rule("a.b", { isRegex: true })]);
    expect(highlightMatches("a.b axb", compiled).map((match) => match.start)).toEqual([0, 4]);
  });

  it("cannot tell case apart unless the rule asks it to", () => {
    expect(highlightMatches("Timeout timeout", compileHighlightRules([rule("TIMEOUT")]))).toHaveLength(2);
    expect(
      highlightMatches("Timeout timeout", compileHighlightRules([rule("TIMEOUT", { caseSensitive: true })])),
    ).toHaveLength(0);
  });

  it("skips a pattern that does not compile, and keeps the rest", () => {
    const compiled = compileHighlightRules([rule("(", { isRegex: true }), rule("timeout")]);
    expect(compiled).toHaveLength(1);
    expect(highlightMatches("timeout", compiled)).toHaveLength(1);
  });

  it("never puts a mark on an empty match, and bounds one line", () => {
    const compiled = compileHighlightRules([rule("x*", { isRegex: true })]);
    expect(highlightMatches("abc", compiled)).toEqual([]);

    const everyLetter = compileHighlightRules([rule(".", { isRegex: true })]);
    expect(highlightMatches("abcdefghijklmnopqrstuvwxyz", everyLetter)).toHaveLength(20);
  });

  it("carries the rule's colour and id on every mark", () => {
    const compiled = compileHighlightRules([rule("timeout", { color: "#ff0000" })]);
    expect(highlightMatches("timeout", compiled)[0]).toMatchObject({ color: "#ff0000", ruleId: "timeout" });
  });
});
