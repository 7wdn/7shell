/**
 * What the sidebar draws, as a list of rows.
 *
 * The interesting parts are not the happy path but the three things here that are decisions rather
 * than consequences: sessions that are in no group are drawn at the root with no heading over them,
 * a group that a search asked for is drawn open whatever its own saved state is, and a group that is
 * empty says so once — which is the difference between "nothing here yet" and "something is broken".
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
      default:
        return `${"  ".repeat(row.depth)}·${row.scope}`;
    }
  });
}

describe("the rows the sidebar draws", () => {
  it("draws a group before the sessions in it, and its own groups before those", () => {
    const rows = visibleRows([work, lab, rack], [web, db, cache], none);

    expect(shape(rows)).toEqual(["Work", "  Rack", "    cache", "  db", "Lab", "  ·group", "web"]);
  });

  it("hides what is under a closed group and keeps the group itself", () => {
    const rows = visibleRows([work, lab, rack], [web, db, cache], new Set([work.id]));

    expect(shape(rows)).toEqual(["Work (closed)", "Lab", "  ·group", "web"]);
  });

  it("draws a group open when it was asked for, whatever its own state is", () => {
    const rows = visibleRows([work], [db], new Set([work.id]), new Set([work.id]));

    expect(shape(rows)).toEqual(["Work", "  db"]);
  });

  it("says a group is empty once, where its contents would have been", () => {
    const rows = visibleRows([lab], [web], none);

    expect(shape(rows)).toEqual(["Lab", "  ·group", "web"]);
  });

  it("does not say a group is empty when it holds only groups", () => {
    const rows = visibleRows([work, rack], [], none);

    expect(shape(rows)).toEqual(["Work", "  Rack", "    ·group"]);
  });

  it("draws a session that is in no group at the root, with no heading and no indent", () => {
    const rows = visibleRows([work], [web, db], none);

    expect(shape(rows)).toEqual(["Work", "  db", "web"]);
    expect(rows[2] && rows[2].kind === "session" ? rows[2].depth : -1).toBe(0);
  });

  it("draws a flat list when there is no group at all", () => {
    const rows = visibleRows([], [web], none);

    expect(shape(rows)).toEqual(["web"]);
  });

  it("keeps a session whose group is gone at the root rather than losing it", () => {
    const orphan = session("session-9", "orphan", "group-that-is-gone");
    const rows = visibleRows([work], [orphan], none);

    expect(shape(rows)).toEqual(["Work", "  ·group", "orphan"]);
  });

  it("hands a drag the thing each row stands for", () => {
    const rows = visibleRows([work, lab], [db], none);
    const targets = rows.map((row) => rowTarget(row));

    expect(shape(rows)).toEqual(["Work", "  db", "Lab", "  ·group"]);
    expect(targets[0]).toEqual({ kind: "group", id: work.id });
    expect(targets[1]).toEqual({ kind: "session", parentId: work.id });
    expect(targets[2]).toEqual({ kind: "group", id: lab.id });
    // The hint is a line of text, not something a drag can land on.
    expect(targets[3]).toBeNull();
  });

  it("hands a drag the root when the row it is on is in no group", () => {
    const rows = visibleRows([work], [web], none);
    const loose = rows.find((row) => row.kind === "session");

    expect(loose && rowTarget(loose)).toEqual({ kind: "session", parentId: null });
  });
});
