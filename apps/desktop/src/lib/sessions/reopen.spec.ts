/**
 * A session opened more than once: what the menu does about it, and what the second tab is called.
 *
 * Two rules, and both are read here rather than inferred from a store that is halfway through
 * opening something: nothing is opened when nothing is open (whatever was remembered), and the
 * numbering is about copies of the *same* thing — a file manager beside a terminal is not a second
 * terminal and must not be numbered like one.
 */
import { describe, expect, it } from "vitest";

import { type SessionTabLike, reopenAction, sessionCopyIndex, sessionTabTitle } from "./reopen";

describe("reopenAction", () => {
  it("opens a session that is not open, whatever was remembered about opening it again", () => {
    // The question is about a second tab; asking it before the first one exists would be asking
    // about a tab that is not there.
    expect(reopenAction(0, "ask")).toBe("open");
    expect(reopenAction(0, "new-tab")).toBe("open");
    expect(reopenAction(0, "focus")).toBe("open");
  });

  it("opens another tab when that is what was remembered", () => {
    expect(reopenAction(1, "new-tab")).toBe("open");
    expect(reopenAction(3, "new-tab")).toBe("open");
  });

  it("goes to the tab that is open when that is what was remembered", () => {
    expect(reopenAction(1, "focus")).toBe("focus");
  });

  it("asks when nothing has been remembered yet", () => {
    expect(reopenAction(1, "ask")).toBe("ask");
    expect(reopenAction(2, "ask")).toBe("ask");
  });
});

describe("sessionCopyIndex", () => {
  const terminal = (id: string, sessionId = "lab"): SessionTabLike => ({ id, sessionId, kind: "terminal" });
  const browser = (id: string, sessionId = "lab"): SessionTabLike => ({ id, sessionId, kind: "files" });

  it("counts the first tab of a session as no copy at all", () => {
    expect(sessionCopyIndex([terminal("t1")], ["t1"])).toEqual({ t1: 0 });
  });

  it("numbers the tabs after the first, in the order they were opened", () => {
    expect(sessionCopyIndex([terminal("t1"), terminal("t2"), terminal("t3")], ["t1", "t2", "t3"])).toEqual({
      t1: 0,
      t2: 1,
      t3: 2,
    });
  });

  it("reads the opening order rather than the order the tabs sit in", () => {
    // The array is what a drag changes; a dragged tab is still the same tab and must keep its
    // number.
    expect(sessionCopyIndex([terminal("t2"), terminal("t1")], ["t1", "t2"])).toEqual({ t1: 0, t2: 1 });
  });

  it("does not number the file manager as a second terminal", () => {
    expect(sessionCopyIndex([terminal("t1"), browser("f1")], ["t1", "f1"])).toEqual({ t1: 0, f1: 0 });
  });

  it("counts each session's tabs on their own", () => {
    const tabs = [terminal("a1", "lab"), terminal("b1", "web"), terminal("a2", "lab")];
    expect(sessionCopyIndex(tabs, ["a1", "b1", "a2"])).toEqual({ a1: 0, b1: 0, a2: 1 });
  });

  it("collapses the numbers when a tab in the middle is closed", () => {
    // The store hands over the tabs that are left, so what the third tab was called is read off
    // what is there now: closing the first copy makes the second the first.
    expect(sessionCopyIndex([terminal("t2"), terminal("t3")], ["t2", "t3"])).toEqual({ t2: 0, t3: 1 });
  });

  it("leaves a tab it was not handed out of the numbering", () => {
    expect(sessionCopyIndex([terminal("t1")], ["t1", "gone"])).toEqual({ t1: 0 });
  });
});

describe("sessionTabTitle", () => {
  it("keeps the session's own name for the first tab", () => {
    expect(sessionTabTitle("lab", 0)).toBe("lab");
  });

  it("numbers every copy after it from one", () => {
    expect(sessionTabTitle("lab", 1)).toBe("lab (1)");
    expect(sessionTabTitle("lab", 2)).toBe("lab (2)");
  });
});
