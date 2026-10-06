/**
 * What a saved session, a group in the list and a running forward offer.
 *
 * The session menu is the sidebar's own buttons plus the address, which is the thing somebody
 * actually wants out of a session list — and the address is printed exactly the way the row prints
 * it, so what is copied is what was read.
 *
 * The group half is about what a menu must not promise: a session cannot be moved into a library
 * with no groups, a group at the top of the list cannot be moved up, and neither is offered as a
 * row that would do nothing.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText } from "./clipboard";
import { groupMenu, sessionEndpoint, sessionMenu, tunnelMenu } from "./sessions";
import type { MenuAction, MenuEntry, MenuSubmenu } from "./types";
import type { OpenTunnel, Session, SessionGroup } from "@/types/session";

vi.mock("./clipboard", () => ({ copyText: vi.fn(async () => true) }));

const session: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 2222,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
  updatedAt: 1_700_000_000,
};

const work: SessionGroup = { id: "group-1", name: "Work", order: 0 };
const lab: SessionGroup = { id: "group-2", name: "Lab", order: 1 };

const tunnel: OpenTunnel = {
  id: "tunnel-1",
  handle: "t1",
  sessionId: session.id,
  bindHost: "127.0.0.1",
  bindPort: 8080,
  mode: "local",
  targetHost: "db.internal",
  targetPort: 5432,
  connections: 0,
};

function pick(entries: readonly MenuEntry[], id: string): MenuAction {
  const found = entries.find((entry): entry is MenuAction => entry.kind === "action" && entry.id === id);
  if (!found) {
    throw new Error(`the menu has no entry called ${id}`);
  }
  return found;
}

function submenu(entries: readonly MenuEntry[], id: string): MenuSubmenu {
  const found = entries.find((entry): entry is MenuSubmenu => entry.kind === "submenu" && entry.id === id);
  if (!found) {
    throw new Error(`the menu has no submenu called ${id}`);
  }
  return found;
}

function sessionActions() {
  return {
    openTerminal: vi.fn(),
    openFiles: vi.fn(),
    edit: vi.fn(),
    remove: vi.fn(),
    move: vi.fn(),
    newGroup: vi.fn(),
  };
}

beforeEach(() => {
  vi.mocked(copyText).mockClear();
});

describe("a saved session's menu", () => {
  it("opens both halves, edits, deletes and copies the address", () => {
    const entries = sessionMenu(session, [], sessionActions());

    expect(entries.filter((entry) => entry.kind === "action").map((entry) => entry.id)).toEqual([
      "open-terminal",
      "open-files",
      "edit",
      "move-to-new-group",
      "remove",
      "copy-endpoint",
    ]);
    expect(pick(entries, "remove").danger).toBe(true);
  });

  it("names the session in the two entries that open something", () => {
    const entries = sessionMenu(session, [], sessionActions());

    expect(pick(entries, "open-terminal").params).toEqual({ name: "web" });
    expect(pick(entries, "open-files").params).toEqual({ name: "web" });
  });

  it("hands each entry the session it belongs to", () => {
    const wired = sessionActions();
    const entries = sessionMenu(session, [], wired);

    pick(entries, "open-terminal").run();
    pick(entries, "open-files").run();
    pick(entries, "edit").run();
    pick(entries, "remove").run();

    expect(wired.openTerminal).toHaveBeenCalledWith(session);
    expect(wired.openFiles).toHaveBeenCalledWith(session);
    expect(wired.edit).toHaveBeenCalledWith(session);
    expect(wired.remove).toHaveBeenCalledWith(session);
  });

  it("copies the address the way the row writes it", () => {
    expect(sessionEndpoint(session)).toBe("root@example.test:2222");

    pick(sessionMenu(session, [], sessionActions()), "copy-endpoint").run();

    expect(copyText).toHaveBeenCalledWith("root@example.test:2222");
  });
});

describe("moving a session into a group", () => {
  it("offers making a group when there is none to move into, without a submenu in the way", () => {
    const wired = sessionActions();
    const entries = sessionMenu(session, [], wired);

    expect(entries.some((entry) => entry.kind === "submenu")).toBe(false);
    // A submenu holding one entry and a separator is a door in front of a door, so with no groups
    // the row is drawn where the submenu would have been — and says what it does to the session.
    expect(pick(entries, "move-to-new-group").labelKey).toBe("sessions.list.moveToNewGroup");

    pick(entries, "move-to-new-group").run();
    expect(wired.newGroup).toHaveBeenCalledWith(session);
  });

  it("lists no group first, then the groups in the order the tree draws them, each by its path", () => {
    const rack: SessionGroup = { id: "group-3", name: "Rack", order: 0, parentId: work.id };
    const entries = sessionMenu(session, [lab, work, rack], sessionActions());
    const move = submenu(entries, "move-to-group");

    expect(move.entries.map((entry) => (entry.kind === "action" ? entry.id : entry.kind))).toEqual([
      "move-to-ungrouped",
      "move-to-group-1",
      "move-to-group-3",
      "move-to-group-2",
      "separator",
      "move-to-new-group",
    ]);
    // Each group's row says the group's own name; there is no sentence to translate.
    expect(pick(move.entries, "move-to-group-2").labelKey).toBe("sessions.list.groupLabel");
    expect(pick(move.entries, "move-to-group-2").params).toEqual({ name: "Lab" });
    // A nested group is named by its path: two groups may share a name at different depths.
    expect(pick(move.entries, "move-to-group-3").params).toEqual({ name: "Work / Rack" });
    // And the last line of the list is where a group that is not there yet comes from: it says
    // "New group", because the list it sits at the end of is a list of places to move to.
    expect(pick(move.entries, "move-to-new-group").labelKey).toBe("sessions.list.groupNewForSession");
  });

  it("makes a group for the session, and moves it in", () => {
    const wired = sessionActions();
    const move = submenu(sessionMenu(session, [work], wired), "move-to-group");

    pick(move.entries, "move-to-new-group").run();

    expect(wired.newGroup).toHaveBeenCalledWith(session);
    expect(wired.move).not.toHaveBeenCalled();
  });

  it("shows where the session is now, and moves it where it was sent", () => {
    const wired = sessionActions();
    const inWork = { ...session, groupId: work.id };
    const move = submenu(sessionMenu(inWork, [work, lab], wired), "move-to-group");

    expect(pick(move.entries, "move-to-group-1").checked).toBe(true);
    expect(pick(move.entries, "move-to-group-2").checked).toBe(false);
    expect(pick(move.entries, "move-to-ungrouped").checked).toBe(false);

    pick(move.entries, "move-to-group-2").run();
    pick(move.entries, "move-to-ungrouped").run();

    expect(wired.move).toHaveBeenNthCalledWith(1, inWork, "group-2");
    expect(wired.move).toHaveBeenNthCalledWith(2, inWork, undefined);
  });

  it("reads a session whose group is gone as one that is in no group", () => {
    const orphan = { ...session, groupId: "group-that-is-gone" };
    const move = submenu(sessionMenu(orphan, [work], sessionActions()), "move-to-group");

    expect(pick(move.entries, "move-to-ungrouped").checked).toBe(true);
    expect(pick(move.entries, "move-to-group-1").checked).toBe(false);
  });
});

describe("a group's menu", () => {
  const ends = { up: true, down: true };

  function groupActions() {
    return { rename: vi.fn(), remove: vi.fn(), move: vi.fn(), addInside: vi.fn() };
  }

  it("renames, nests, moves and deletes — and never anything about the sessions inside", () => {
    const entries = groupMenu(work, groupActions(), ends);

    expect(entries.filter((entry) => entry.kind === "action").map((entry) => entry.id)).toEqual([
      "rename-group",
      "add-group-inside",
      "move-group-up",
      "move-group-down",
      "remove-group",
    ]);
    expect(pick(entries, "remove-group").danger).toBe(true);
  });

  it("hands over the group, the direction a move goes in, and where a nested one goes", () => {
    const wired = groupActions();
    const entries = groupMenu(work, wired, ends);

    pick(entries, "rename-group").run();
    pick(entries, "add-group-inside").run();
    pick(entries, "move-group-up").run();
    pick(entries, "move-group-down").run();
    pick(entries, "remove-group").run();

    expect(wired.rename).toHaveBeenCalledWith(work);
    expect(wired.addInside).toHaveBeenCalledWith(work);
    expect(wired.move).toHaveBeenNthCalledWith(1, work, -1);
    expect(wired.move).toHaveBeenNthCalledWith(2, work, 1);
    expect(wired.remove).toHaveBeenCalledWith(work);
  });

  it("draws the moves that lead nowhere but does not let them be run", () => {
    const entries = groupMenu(work, groupActions(), { up: false, down: true });

    expect(pick(entries, "move-group-up").disabled).toBe(true);
    expect(pick(entries, "move-group-down").disabled).toBe(false);
  });
});

describe("a forward's menu", () => {
  it("offers stopping it and copying where it leads", () => {
    const stop = vi.fn();
    const entries = tunnelMenu(tunnel, { stop });

    expect(pick(entries, "stop").danger).toBe(true);

    pick(entries, "stop").run();
    expect(stop).toHaveBeenCalledWith("tunnel-1");

    pick(entries, "copy-forward").run();
    expect(copyText).toHaveBeenCalledWith("127.0.0.1:8080 → db.internal:5432");
  });
});
