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

function group(id: string, name: string, order: number, parentId?: string, pinned = false): SessionGroup {
  return { id, name, order, parentId, pinned };
}

function session(id: string, name: string, groupId?: string, order?: number): Session {
  return {
    id,
    name,
    groupId,
    order,
    protocol: "ssh",
    host: "example.test",
    port: 22,
    user: "root",
    auth: { method: "password", hasSecret: false },
    terminal: {
      terminalType: "xterm-256color",
      keepaliveSeconds: 30,
      encoding: "utf-8",
      backspace: "del",
      forwardAgent: false,
      record: false,
    },
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
      default:
        return `${"  ".repeat(row.depth)}${row.session.name}`;
    }
  });
}

describe("the rows the sidebar draws", () => {
  it("draws a group before the sessions in it, and its own groups before those", () => {
    const rows = visibleRows([work, lab, rack], [web, db, cache], none);

    expect(shape(rows)).toEqual(["Work", "  Rack", "    cache", "  db", "Lab", "web"]);
  });

  it("hides what is under a closed group and keeps the group itself", () => {
    const rows = visibleRows([work, lab, rack], [web, db, cache], new Set([work.id]));

    expect(shape(rows)).toEqual(["Work (closed)", "Lab", "web"]);
  });

  it("draws a group open when it was asked for, whatever its own state is", () => {
    const rows = visibleRows([work], [db], new Set([work.id]), new Set([work.id]));

    expect(shape(rows)).toEqual(["Work", "  db"]);
  });

  it("draws nothing under a group with nothing in it", () => {
    // An empty folder is a folder. A line saying so is one more line to read in a list somebody is
    // scanning, and the reference draws nothing here either.
    const rows = visibleRows([lab], [web], none);

    expect(shape(rows)).toEqual(["Lab", "web"]);
  });

  it("draws nothing under a group that holds only empty groups", () => {
    const rows = visibleRows([work, rack], [], none);

    expect(shape(rows)).toEqual(["Work", "  Rack"]);
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

    expect(shape(rows)).toEqual(["Work", "orphan"]);
  });

  it("draws the sessions of a group in the order they were dragged into, ties by creation", () => {
    const late = session("session-4", "late", work.id, 2);
    const early = session("session-5", "early", work.id, 1);
    const unplaced = session("session-6", "unplaced", work.id);

    const rows = visibleRows([work], [late, unplaced, early, db], none);

    // `db` carries no order, so it is level with zero: before the placed ones, in creation order.
    expect(shape(rows)).toEqual(["Work", "  unplaced", "  db", "  early", "  late"]);
  });

  it("draws a session between two groups when the arrangement says so", () => {
    const between = session("session-7", "between", undefined, 1);
    const spaced = [
      { ...work, order: 0 },
      { ...lab, order: 2 },
    ];

    const rows = visibleRows(spaced, [between], none);

    expect(shape(rows)).toEqual(["Work", "between", "Lab"]);
  });

  it("draws a group between two sessions when the arrangement says so", () => {
    const first = session("session-8", "first", undefined, 0);
    const second = session("session-9", "second", undefined, 2);
    const between = { ...work, order: 1 };

    const rows = visibleRows([between], [first, second], none);

    expect(shape(rows)).toEqual(["first", "Work", "second"]);
  });

  it("hands a drag the thing each row stands for", () => {
    const rows = visibleRows([work, lab], [db], none);
    const targets = rows.map((row) => rowTarget(row));

    expect(shape(rows)).toEqual(["Work", "  db", "Lab"]);
    expect(targets[0]).toEqual({ kind: "group", id: work.id });
    expect(targets[1]).toEqual({ kind: "session", id: "session-2", parentId: work.id });
    expect(targets[2]).toEqual({ kind: "group", id: lab.id });
  });

  it("hands a drag the root when the row it is on is in no group", () => {
    const rows = visibleRows([work], [web], none);
    const loose = rows.find((row) => row.kind === "session");

    expect(loose && rowTarget(loose)).toEqual({ kind: "session", id: "session-1", parentId: null });
  });
});
