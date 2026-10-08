import { describe, expect, it } from "vitest";

import { matchText, NO_MATCH } from "@/lib/commands/match";

describe("how well a candidate answers", () => {
  it("orders the tiers: exact, then initials, then prefix, then a run inside, then letters apart", () => {
    // Initials sit above a prefix because of what they mean: `gc` answering `git commit` is two
    // letters landing on the first letters of two words, which is a stronger signal than the same
    // two letters starting one word.
    const exact = matchText("sed", "sed").score;
    const initials = matchText("system device", "sd").score;
    const prefix = matchText("sedtool", "sed").score;
    const inside = matchText("lvsedit", "sed").score;
    const scattered = matchText("s---e---d", "sed").score;

    expect(exact).toBeGreaterThan(initials);
    expect(initials).toBeGreaterThan(prefix);
    expect(prefix).toBeGreaterThan(inside);
    expect(inside).toBeGreaterThan(scattered);
  });

  it("lets the shorter candidate win inside a tier", () => {
    expect(matchText("ls", "l").score).toBeGreaterThan(matchText("lsof", "l").score);
  });

  it("matches the initials of the words, and says which letters they were", () => {
    expect(matchText("git commit", "gc")).toMatchObject({
      ranges: [
        { start: 0, end: 1 },
        { start: 4, end: 5 },
      ],
    });
  });

  it("rewards a match at the start of a word over one in the middle of it", () => {
    expect(matchText("git push", "push").score).toBeGreaterThan(matchText("repushing", "push").score);
  });

  it("charges for the gaps between scattered letters", () => {
    expect(matchText("grep", "gp").score).toBeGreaterThan(matchText("g-----p", "gp").score);
  });

  it("answers nothing for letters that are not there", () => {
    expect(matchText("grep", "xyz")).toEqual({ score: NO_MATCH, ranges: [] });
  });

  it("answers everything for an empty prefix, so an empty word lists what there is", () => {
    expect(matchText("grep", "")).toEqual({ score: 0, ranges: [] });
  });

  it("gives back the characters the score was made of", () => {
    expect(matchText("grep", "gr").ranges).toEqual([{ start: 0, end: 2 }]);
    expect(matchText("grep", "gp").ranges).toEqual([
      { start: 0, end: 1 },
      { start: 3, end: 4 },
    ]);
  });

  it("ignores case when it matches, and remembers that it was exact", () => {
    expect(matchText("Grep", "grep").score).toBeLessThan(matchText("grep", "grep").score);
    expect(matchText("Grep", "grep").score).toBeGreaterThan(matchText("grep tool", "grep").score);
  });
});
