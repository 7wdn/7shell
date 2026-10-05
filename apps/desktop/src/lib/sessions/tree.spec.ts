/**
 * What the sidebar draws, as a list of rows.
 *
 * The interesting part is not the happy path but the two rows that exist for reasons of their own:
 * the bucket for sessions that are in no group — drawn as soon as there is a group to be outside of,
 * because it is a place a drag can be dropped on and a place that comes and goes is a place nobody
 * can aim at — and the hint that says a group is empty, which is the difference between "nothing
 * here yet" and "something is broken".
 */
import { describe, expect, it } from "vitest";

import { rowTarget, visibleRows } from "./tree";
import type { Session, SessionGroup } from "@/types/session";

function group(id: string, name: string, order: number, parentId?: string): SessionGroup {
  return { id, name, order, parentId };
}

function session(id: string, name: string, groupId?: string): Session {
  return {
    id,
    name,
    groupId,
    protocol: "ssh",
    host: "example.test",
    port: 22,
    user: "root",
    auth: { method: "password", hasSecret: false },
    terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
    updatedAt: 1_700_000_000,
  };
}

const work = group("group-1", "Work", 0);
const lab = group("group-2", "Lab", 1);
const rack = group("group-3", "Rack", 0, work.id);
const web = session("session-1", "web");
const db = session("session-2", "db", work.id);
const cache = session("session-3", "cache", rack.id);

const none: ReadonlySet<string> = new Set();

/** The rows as a short line each, which is what a reader of the tree actually sees. */
function shape(rows: ReturnType<typeof visibleRows>): string[] {
  return rows.map((row) => {
    switch (row.kind) {
      case "group":
        return `${"  ".repeat(row.depth)}${row.group.name}${row.collapsed ? " (closed)" : ""}`;
      case "session":
        return `${"  ".repeat(row.depth)}${row.session.name}`;
      case "ungrouped":
        return "Ungrouped";
      default:
        return `${"  ".repeat(row.depth)}·${row.scope}`;
    }
  });
}

describe("the rows the sidebar draws", () => {
  it("draws a group before the sessions in it, and its own groups before those", () => {
    const rows = visibleRows([work, lab, rack], [web, db, cache], none);

    expect(shape(rows)).toEqual(["Work", "  Rack", "    cache", "  db", "Lab", "  ·group", "Ungrouped", "  web"]);
  });

  it("hides what is under a closed group and keeps the group itself", () => {
    const rows = visibleRows([work, lab, rack], [web, db, cache], new Set([work.id]));

    expect(shape(rows)).toEqual(["Work (closed)", "Lab", "  ·group", "Ungrouped", "  web"]);
  });

  it("says a group is empty once, where its contents would have been", () => {
    const rows = visibleRows([lab], [web], none);

    expect(shape(rows)).toEqual(["Lab", "  ·group", "Ungrouped", "  web"]);
  });

  it("does not say a group is empty when it holds only groups", () => {
    const rows = visibleRows([work, rack], [], none);

    expect(shape(rows)).toEqual(["Work", "  Rack", "    ·group", "Ungrouped", "  ·ungrouped"]);
  });

  it("draws no bucket at all when there is no group to be outside of", () => {
    const rows = visibleRows([], [web], none);

    expect(shape(rows)).toEqual(["web"]);
  });

  it("draws the bucket as soon as a group exists, even with nothing loose in it", () => {
    const rows = visibleRows([work], [db], none);

    expect(shape(rows)).toEqual(["Work", "  db", "Ungrouped", "  ·ungrouped"]);
  });

  it("keeps a session whose group is gone in the bucket rather than losing it", () => {
    const orphan = session("session-9", "orphan", "group-that-is-gone");
    const rows = visibleRows([work], [orphan], none);

    expect(shape(rows)).toEqual(["Work", "  ·group", "Ungrouped", "  orphan"]);
  });

  it("hands a drag the thing each row stands for", () => {
    const rows = visibleRows([work, lab], [db, { ...cache, groupId: lab.id }], none);
    const targets = rows.map((row) => rowTarget(row));

    expect(shape(rows)).toEqual(["Work", "  db", "Lab", "  cache", "Ungrouped", "  ·ungrouped"]);
    expect(targets[0]).toEqual({ kind: "group", id: work.id });
    expect(targets[1]).toEqual({ kind: "session", parentId: work.id });
    expect(targets[2]).toEqual({ kind: "group", id: lab.id });
    expect(targets[3]).toEqual({ kind: "session", parentId: lab.id });
    expect(targets[4]).toEqual({ kind: "ungrouped" });
    // The hint is a line of text, not something a drag can land on.
    expect(targets[5]).toBeNull();
  });
});
