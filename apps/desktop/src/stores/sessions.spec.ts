import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionsList: vi.fn(),
  sessionSave: vi.fn(),
  sessionDelete: vi.fn(),
  groupsList: vi.fn(),
  groupSave: vi.fn(),
  groupDelete: vi.fn(),
}));

import { groupDelete, groupSave, groupsList, sessionDelete, sessionSave, sessionsList } from "@/lib/backend/tauri";
import { useSessionsStore } from "@/stores/sessions";
import type { Session, SessionGroup } from "@/types/session";

const sessionsListMock = vi.mocked(sessionsList);
const sessionSaveMock = vi.mocked(sessionSave);
const sessionDeleteMock = vi.mocked(sessionDelete);
const groupsListMock = vi.mocked(groupsList);
const groupSaveMock = vi.mocked(groupSave);
const groupDeleteMock = vi.mocked(groupDelete);

const draft: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: false },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
  updatedAt: 1_700_000_000,
};

const work: SessionGroup = { id: "group-1", name: "Work", order: 0 };
const lab: SessionGroup = { id: "group-2", name: "Lab", order: 1 };
const rack: SessionGroup = { id: "group-3", name: "Rack", order: 0, parentId: work.id };

/** The rows as short lines — the tree the sidebar draws, without the drawing. */
function shape(store: ReturnType<typeof useSessionsStore>): string[] {
  return store.rows.map((row) => {
    switch (row.kind) {
      case "group":
        return `${"  ".repeat(row.depth)}${row.group.name}`;
      case "session":
        return `${"  ".repeat(row.depth)}${row.session.name}`;
      case "ungrouped":
        return "Ungrouped";
      default:
        return `${"  ".repeat(row.depth)}·${row.scope === "group" ? "empty" : "none"}`;
    }
  });
}

/** The envelope a refused call comes back with, which is what the sidebar draws. */
function refusal(code: string) {
  return {
    version: 1,
    code,
    messageKey: "errors.store.io",
    messageParams: {},
    origin: { subsystem: "store" },
    outcome: "rejected",
  };
}

describe("sessions store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    sessionsListMock.mockReset().mockResolvedValue([]);
    groupsListMock.mockReset().mockResolvedValue([]);
    sessionSaveMock.mockReset().mockImplementation(async (session: Session) => session);
    sessionDeleteMock.mockReset().mockResolvedValue(undefined);
    groupSaveMock.mockReset().mockImplementation(async (group: SessionGroup) => group);
    groupDeleteMock.mockReset().mockResolvedValue(undefined);
  });

  it("sends no secret when nobody typed one, so the stored one is left alone", async () => {
    const store = useSessionsStore();

    await store.save(draft);

    expect(sessionSaveMock).toHaveBeenCalledWith(draft, undefined);
  });

  it("sends the secret that was typed", async () => {
    const store = useSessionsStore();

    await store.save(draft, { value: "hunter2" });

    expect(sessionSaveMock).toHaveBeenCalledWith(draft, { value: "hunter2" });
  });

  it("keeps the backend's answer, which is where the truth about a stored secret comes from", async () => {
    sessionSaveMock.mockResolvedValue({ ...draft, auth: { method: "password", hasSecret: true } });

    const store = useSessionsStore();
    await store.save(draft, { value: "hunter2" });

    expect(store.sessions).toHaveLength(1);
    expect(store.sessions[0]?.auth.hasSecret).toBe(true);
  });

  it("replaces an edited session in place rather than adding a second copy", async () => {
    sessionsListMock.mockResolvedValue([draft]);

    const store = useSessionsStore();
    await store.load();
    await store.save({ ...draft, name: "web renamed" });

    expect(store.sessions).toHaveLength(1);
    expect(store.sessions[0]?.name).toBe("web renamed");
  });

  it("removes a deleted session from the list", async () => {
    sessionsListMock.mockResolvedValue([draft, { ...draft, id: "session-2", name: "db" }]);

    const store = useSessionsStore();
    await store.load();
    await store.remove("session-1");

    expect(sessionDeleteMock).toHaveBeenCalledWith("session-1");
    expect(store.sessions.map((session) => session.id)).toEqual(["session-2"]);
  });

  it("keeps a deletion failure as an envelope the sidebar can show", async () => {
    sessionsListMock.mockResolvedValue([draft]);
    sessionDeleteMock.mockRejectedValue(refusal("STORE-IO"));

    const store = useSessionsStore();
    await store.load();
    await store.remove("session-1");

    expect(store.failure?.code).toBe("STORE-IO");
    expect(store.sessions).toHaveLength(1);
  });

  it("groups the list, and keeps a session whose group is no longer saved", async () => {
    groupsListMock.mockResolvedValue([{ id: "group-1", name: "Work", order: 0 }]);
    sessionsListMock.mockResolvedValue([
      { ...draft, id: "session-1", groupId: "group-1" },
      { ...draft, id: "session-2", groupId: "group-that-is-gone" },
      { ...draft, id: "session-3" },
    ]);

    const store = useSessionsStore();
    await store.load();

    expect(shape(store)).toEqual(["Work", "  web", "Ungrouped", "  web", "  web"]);
  });

  it("nests what a group holds, in the order the tree draws it", async () => {
    groupsListMock.mockResolvedValue([work, lab, rack]);
    sessionsListMock.mockResolvedValue([{ ...draft, id: "session-1", name: "inside", groupId: rack.id }]);

    const store = useSessionsStore();
    await store.load();

    expect(shape(store)).toEqual(["Work", "  Rack", "    inside", "Lab", "  ·empty", "Ungrouped", "  ·none"]);
  });

  it("keeps what is under a closed group out of the rows, and puts it back when it opens", async () => {
    groupsListMock.mockResolvedValue([work, rack]);
    sessionsListMock.mockResolvedValue([{ ...draft, id: "session-1", name: "inside", groupId: rack.id }]);

    const store = useSessionsStore();
    await store.load();
    store.setCollapsed(work.id, true);

    expect(shape(store)).toEqual(["Work", "Ungrouped", "  ·none"]);

    store.setCollapsed(work.id, false);

    expect(shape(store)).toEqual(["Work", "  Rack", "    inside", "Ungrouped", "  ·none"]);
  });
});

describe("making and changing groups", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    sessionsListMock.mockReset().mockResolvedValue([]);
    groupsListMock.mockReset().mockResolvedValue([]);
    sessionSaveMock.mockReset().mockImplementation(async (session: Session) => session);
    groupSaveMock.mockReset().mockImplementation(async (group: SessionGroup) => group);
    groupDeleteMock.mockReset().mockResolvedValue(undefined);
  });

  it("makes a group with a fresh id, placed after the ones that already exist", async () => {
    groupsListMock.mockResolvedValue([{ ...work, order: 4 }]);

    const store = useSessionsStore();
    await store.load();
    const worked = await store.addGroup("Lab");

    expect(worked).toBe(true);
    const written = groupSaveMock.mock.calls[0]?.[0];
    expect(written?.name).toBe("Lab");
    expect(written?.order).toBe(5);
    expect(written?.id).toBeTruthy();
    expect(store.groups.map((group) => group.name)).toEqual(["Work", "Lab"]);
  });

  it("makes a group inside another one, and opens it so the new group can be seen", async () => {
    groupsListMock.mockResolvedValue([work, rack]);

    const store = useSessionsStore();
    await store.load();
    store.setCollapsed(work.id, true);

    const worked = await store.addGroup("Shelf", work.id);

    expect(worked).toBe(true);
    expect(groupSaveMock.mock.calls[0]?.[0]).toMatchObject({ name: "Shelf", parentId: work.id, order: 1 });
    expect(store.collapsed.has(work.id)).toBe(false);
    expect(shape(store)).toContain("  Shelf");
  });

  it("keeps a group write failure as an envelope and adds nothing to the list", async () => {
    groupSaveMock.mockRejectedValue(refusal("STORE-IO"));

    const store = useSessionsStore();
    const worked = await store.addGroup("Lab");

    expect(worked).toBe(false);
    expect(store.failure?.code).toBe("STORE-IO");
    expect(store.groups).toEqual([]);
  });

  it("replaces a renamed group in place rather than adding a second one", async () => {
    groupsListMock.mockResolvedValue([work]);
    groupSaveMock.mockImplementation(async (group: SessionGroup) => group);

    const store = useSessionsStore();
    await store.load();
    await store.saveGroup({ ...work, name: "Work (EU)" });

    expect(store.groups).toHaveLength(1);
    expect(store.groups[0]?.name).toBe("Work (EU)");
  });

  it("deletes the group and nothing else: its sessions move out to ungrouped", async () => {
    groupsListMock.mockResolvedValue([work, lab]);
    sessionsListMock.mockResolvedValue([{ ...draft, id: "session-1", groupId: work.id }]);

    const store = useSessionsStore();
    await store.load();
    const worked = await store.removeGroup(work.id);

    expect(worked).toBe(true);
    expect(groupDeleteMock).toHaveBeenCalledWith(work.id);
    expect(store.groups.map((group) => group.id)).toEqual([lab.id]);
    // The session is still here, and now it sits under Ungrouped — the invariant the confirmation
    // in the sidebar says out loud.
    expect(shape(store)).toEqual(["Lab", "  ·empty", "Ungrouped", "  web"]);
    expect(store.sessions[0]?.groupId).toBeUndefined();
  });

  it("keeps a group deletion failure as an envelope and leaves the list alone", async () => {
    groupsListMock.mockResolvedValue([work]);
    groupDeleteMock.mockRejectedValue(refusal("STORE-IO"));

    const store = useSessionsStore();
    await store.load();
    const worked = await store.removeGroup(work.id);

    expect(worked).toBe(false);
    expect(store.failure?.code).toBe("STORE-IO");
    expect(store.groups).toHaveLength(1);
  });

  it("moves a nested group out to the root with it, the way the vault does", async () => {
    // The vault lifts the children out of a group it deletes, and the model nests — so the copy on
    // screen has to lift them out the same way, or the next read would come back looking different
    // from what was drawn in the meantime.
    const child: SessionGroup = { id: "group-3", name: "Inside", order: 0, parentId: work.id };
    groupsListMock.mockResolvedValue([work, child]);

    const store = useSessionsStore();
    await store.load();
    await store.removeGroup(work.id);

    expect(store.groups.map((group) => [group.id, group.parentId])).toEqual([["group-3", undefined]]);
    expect(shape(store)).toEqual(["Inside", "  ·empty", "Ungrouped", "  ·none"]);
  });

  it("writes only the two groups whose order changed when one is moved a place", async () => {
    groupsListMock.mockResolvedValue([work, lab]);

    const store = useSessionsStore();
    await store.load();
    await store.moveGroupBy(lab.id, -1);

    expect(groupSaveMock.mock.calls.map(([group]) => [group.id, group.order])).toEqual([
      ["group-2", 0],
      ["group-1", 1],
    ]);
    expect(shape(store)).toEqual(["Lab", "  ·empty", "Work", "  ·empty", "Ungrouped", "  ·none"]);
  });

  it("moves a group among its own siblings, never past a group that is not one", async () => {
    groupsListMock.mockResolvedValue([work, lab, rack]);

    const store = useSessionsStore();
    await store.load();
    // Rack is the only child of Work, so there is nowhere for it to go — even though it is not the
    // last group in the library.
    const worked = await store.moveGroupBy(rack.id, 1);

    expect(worked).toBe(false);
    expect(groupSaveMock).not.toHaveBeenCalled();
  });

  it("writes nothing when a move has nowhere to go", async () => {
    groupsListMock.mockResolvedValue([work, lab]);

    const store = useSessionsStore();
    await store.load();
    const worked = await store.moveGroupBy(work.id, -1);

    expect(worked).toBe(false);
    expect(groupSaveMock).not.toHaveBeenCalled();
  });

  it("places a group inside another one through a drop, at the end of what is there", async () => {
    groupsListMock.mockResolvedValue([work, lab]);

    const store = useSessionsStore();
    await store.load();
    const worked = await store.placeGroup(work.id, { kind: "group", id: lab.id }, "inside");

    expect(worked).toBe(true);
    expect(groupSaveMock.mock.calls.map(([group]) => [group.id, group.parentId, group.order])).toEqual([
      ["group-1", "group-2", 0],
    ]);
    expect(shape(store)).toEqual(["Lab", "  Work", "    ·empty", "Ungrouped", "  ·none"]);
  });

  it("refuses a drop that would put a group under itself, and writes nothing", async () => {
    groupsListMock.mockResolvedValue([work, rack]);

    const store = useSessionsStore();
    await store.load();
    const worked = await store.placeGroup(work.id, { kind: "group", id: rack.id }, "inside");

    expect(worked).toBe(false);
    expect(groupSaveMock).not.toHaveBeenCalled();
  });

  it("opens the group a drop landed in, so what was moved can be seen", async () => {
    groupsListMock.mockResolvedValue([work, lab]);

    const store = useSessionsStore();
    await store.load();
    store.setCollapsed(lab.id, true);

    await store.placeGroup(work.id, { kind: "group", id: lab.id }, "inside");

    expect(store.collapsed.has(lab.id)).toBe(false);
  });
});

describe("moving a session between groups", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    sessionsListMock.mockReset().mockResolvedValue([]);
    groupsListMock.mockReset().mockResolvedValue([]);
    sessionSaveMock.mockReset().mockImplementation(async (session: Session) => session);
  });

  it("writes the whole session with its new group and no secret, which keeps the stored one", async () => {
    sessionsListMock.mockResolvedValue([draft]);
    groupsListMock.mockResolvedValue([work]);

    const store = useSessionsStore();
    await store.load();
    const worked = await store.placeSession(draft.id, { kind: "group", id: work.id });

    expect(worked).toBe(true);
    const [written, secret] = sessionSaveMock.mock.calls[0] ?? [];
    expect(written?.groupId).toBe(work.id);
    expect(written?.name).toBe("web");
    expect(secret).toBeUndefined();
    expect(shape(store)).toEqual(["Work", "  web", "Ungrouped", "  ·none"]);
  });

  it("moves a session out of every group when it is sent to the root", async () => {
    sessionsListMock.mockResolvedValue([{ ...draft, groupId: work.id }]);
    groupsListMock.mockResolvedValue([work]);
    sessionSaveMock.mockImplementation(async (session: Session) => session);

    const store = useSessionsStore();
    await store.load();
    await store.placeSession(draft.id, { kind: "root" });

    expect(sessionSaveMock.mock.calls[0]?.[0]?.groupId).toBeUndefined();
    expect(shape(store)).toEqual(["Work", "  ·empty", "Ungrouped", "  web"]);
  });

  it("opens the group a session was dropped into, so it does not vanish into a closed group", async () => {
    sessionsListMock.mockResolvedValue([draft]);
    groupsListMock.mockResolvedValue([work]);

    const store = useSessionsStore();
    await store.load();
    store.setCollapsed(work.id, true);
    await store.placeSession(draft.id, { kind: "group", id: work.id });

    expect(store.collapsed.has(work.id)).toBe(false);
    expect(shape(store)).toEqual(["Work", "  web", "Ungrouped", "  ·none"]);
  });

  it("writes nothing when the session is already where it was dropped", async () => {
    sessionsListMock.mockResolvedValue([{ ...draft, groupId: work.id }]);
    groupsListMock.mockResolvedValue([work]);

    const store = useSessionsStore();
    await store.load();
    const worked = await store.placeSession(draft.id, { kind: "group", id: work.id });

    expect(worked).toBe(false);
    expect(sessionSaveMock).not.toHaveBeenCalled();
  });

  it("keeps the move in the list as the backend answered it", async () => {
    sessionsListMock.mockResolvedValue([draft]);
    groupsListMock.mockResolvedValue([work]);
    sessionSaveMock.mockResolvedValue({ ...draft, groupId: work.id, updatedAt: 1_700_000_999 });

    const store = useSessionsStore();
    await store.load();
    await store.placeSession(draft.id, { kind: "group", id: work.id });

    expect(store.sessions[0]?.updatedAt).toBe(1_700_000_999);
  });

  it("keeps a refused move as an envelope and leaves the session where it was", async () => {
    sessionsListMock.mockResolvedValue([draft]);
    groupsListMock.mockResolvedValue([work]);
    sessionSaveMock.mockRejectedValue(refusal("STORE-IO"));

    const store = useSessionsStore();
    await store.load();
    const worked = await store.placeSession(draft.id, { kind: "group", id: work.id });

    expect(worked).toBe(false);
    expect(store.failure?.code).toBe("STORE-IO");
    expect(store.sessions[0]?.groupId).toBeUndefined();
  });
});
