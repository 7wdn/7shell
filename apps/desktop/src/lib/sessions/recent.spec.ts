/**
 * What the recent list keeps, and what it does with a stored value it did not write.
 *
 * Two rules are under test here: the list is *ordering* rather than membership — an id that the
 * library no longer has must not take up one of the few slots — and a stored value is never
 * trusted: it is somebody's file on disk now, and a list that failed to parse must read as "nothing
 * remembered" rather than as a reason to draw nothing at all.
 */
import { beforeEach, describe, expect, it } from "vitest";

import {
  MAX_RECENT_SESSIONS,
  RECENT_SESSIONS_KEY,
  parseRecentSessionIds,
  rankRecentSessions,
  readRecentSessionIds,
  recordRecentSession,
  writeRecentSessionIds,
} from "./recent";

beforeEach(() => {
  localStorage.clear();
});

describe("parseRecentSessionIds", () => {
  it("reads nothing out of an empty, absent or unparsable value", () => {
    expect(parseRecentSessionIds(null)).toEqual([]);
    expect(parseRecentSessionIds("")).toEqual([]);
    expect(parseRecentSessionIds("not json")).toEqual([]);
    expect(parseRecentSessionIds('{"id":"a"}')).toEqual([]);
    expect(parseRecentSessionIds('"a"')).toEqual([]);
  });

  it("keeps strings in order, drops everything else, and never repeats one", () => {
    expect(parseRecentSessionIds('["a", 7, null, " b ", "a", "", "c"]')).toEqual(["a", "b", "c"]);
  });

  it("stops at the cap, so a list edited by hand cannot outgrow it", () => {
    const many = JSON.stringify(Array.from({ length: 20 }, (_, index) => `s${index}`));
    expect(parseRecentSessionIds(many)).toHaveLength(MAX_RECENT_SESSIONS);
  });
});

describe("recordRecentSession", () => {
  it("puts a new session first and shifts the rest down", () => {
    expect(recordRecentSession(["b", "c"], "a")).toEqual(["a", "b", "c"]);
  });

  it("moves a session that is already there to the front", () => {
    expect(recordRecentSession(["a", "b", "c"], "c")).toEqual(["c", "a", "b"]);
  });

  it("hands back the same list when nothing moved", () => {
    const history = ["a", "b"];
    expect(recordRecentSession(history, "a")).toBe(history);
  });

  it("ignores a blank id rather than writing an empty entry", () => {
    const history = ["a"];
    expect(recordRecentSession(history, "  ")).toBe(history);
  });

  it("forgets the oldest once the list is full", () => {
    const history = recordRecentSession(["b", "c", "d", "e", "f"], "a");
    expect(history).toEqual(["a", "b", "c", "d", "e"]);
    expect(history).not.toContain("f");
  });
});

describe("rankRecentSessions", () => {
  const library = [{ id: "a" }, { id: "b" }, { id: "c" }, { id: "d" }, { id: "e" }, { id: "f" }];

  it("draws the remembered sessions first, in the order they were used", () => {
    expect(rankRecentSessions(library, ["c", "a"]).map((entry) => entry.id)).toEqual(["c", "a", "b", "d", "e"]);
  });

  it("fills the list out from the library rather than leaving it short", () => {
    expect(rankRecentSessions(library, []).map((entry) => entry.id)).toEqual(["a", "b", "c", "d", "e"]);
  });

  it("skips an id the library no longer has without spending a slot on it", () => {
    expect(rankRecentSessions(library, ["gone", "b"]).map((entry) => entry.id)).toEqual(["b", "a", "c", "d", "e"]);
  });

  it("draws nothing at all when the limit is zero", () => {
    expect(rankRecentSessions(library, ["a"], 0)).toEqual([]);
  });

  it("never draws more than the library has", () => {
    expect(rankRecentSessions([{ id: "a" }], ["a"]).map((entry) => entry.id)).toEqual(["a"]);
  });
});

describe("the stored list", () => {
  it("round-trips what it wrote", () => {
    writeRecentSessionIds(["a", "b"]);
    expect(readRecentSessionIds()).toEqual(["a", "b"]);
  });

  it("reads nothing when nothing was written", () => {
    expect(readRecentSessionIds()).toEqual([]);
  });

  it("reads nothing from a value that is not a list of ids", () => {
    localStorage.setItem(RECENT_SESSIONS_KEY, "<html>");
    expect(readRecentSessionIds()).toEqual([]);
  });
});
