import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionsList: vi.fn(),
  sessionSave: vi.fn(),
  sessionDelete: vi.fn(),
  sessionsExport: vi.fn(),
  sessionsImport: vi.fn(),
  groupsList: vi.fn(),
  groupSave: vi.fn(),
  groupDelete: vi.fn(),
}));

import {
  groupDelete,
  groupSave,
  groupsList,
  sessionDelete,
  sessionSave,
  sessionsExport,
  sessionsImport,
  sessionsList,
} from "@/lib/backend/tauri";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore } from "@/stores/terminals";
import type { Session, SessionGroup, SessionImportOutcome } from "@/types/session";

const sessionsListMock = vi.mocked(sessionsList);
const sessionSaveMock = vi.mocked(sessionSave);
const sessionDeleteMock = vi.mocked(sessionDelete);
const sessionsExportMock = vi.mocked(sessionsExport);
const sessionsImportMock = vi.mocked(sessionsImport);
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

const work: SessionGroup = { id: "group-1", name: "Work", order: 0, pinned: false };
const lab: SessionGroup = { id: "group-2", name: "Lab", order: 1, pinned: false };
const rack: SessionGroup = { id: "group-3", name: "Rack", order: 0, parentId: work.id, pinned: false };

/** The rows as short lines — the tree the sidebar draws, without the drawing. */
function shape(store: ReturnType<typeof useSessionsStore>): string[] {
  return store.rows.map((row) => {
    switch (row.kind) {
      case "group":
        return `${"  ".repeat(row.depth)}${row.group.name}`;
      default:
        return `${"  ".repeat(row.depth)}${row.session.name}`;
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
    groupsListMock.mockResolvedValue([{ id: "group-1", name: "Work", order: 0, pinned: false }]);
    sessionsListMock.mockResolvedValue([
      { ...draft, id: "session-1", groupId: "group-1" },
      { ...draft, id: "session-2", groupId: "group-that-is-gone" },
      { ...draft, id: "session-3" },
    ]);

    const store = useSessionsStore();
    await store.load();

    expect(shape(store)).toEqual(["Work", "  web", "web", "web"]);
  });

  it("nests what a group holds, in the order the tree draws it", async () => {
    groupsListMock.mockResolvedValue([work, lab, rack]);
    sessionsListMock.mockResolvedValue([{ ...draft, id: "session-1", name: "inside", groupId: rack.id }]);

    const store = useSessionsStore();
    await store.load();

    expect(shape(store)).toEqual(["Work", "  Rack", "    inside", "Lab"]);
  });

  it("keeps what is under a closed group out of the rows, and puts it back when it opens", async () => {
    groupsListMock.mockResolvedValue([work, rack]);
    sessionsListMock.mockResolvedValue([{ ...draft, id: "session-1", name: "inside", groupId: rack.id }]);

    const store = useSessionsStore();
    await store.load();
    store.setCollapsed(work.id, true);

    expect(shape(store)).toEqual(["Work"]);

    store.setCollapsed(work.id, false);

    expect(shape(store)).toEqual(["Work", "  Rack", "    inside"]);
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
    const made = await store.addGroup("Lab");

    // What comes back is the group the list holds, which is what a caller about to name it needs.
    expect(made?.id).toBeTruthy();
    const written = groupSaveMock.mock.calls[0]?.[0];
    expect(written?.name).toBe("Lab");
    expect(written?.order).toBe(5);
    expect(written?.id).toBe(made?.id);
    expect(store.groups.map((group) => group.name)).toEqual(["Work", "Lab"]);
  });

  it("makes a group inside another one, and opens it so the new group can be seen", async () => {
    groupsListMock.mockResolvedValue([work, rack]);

    const store = useSessionsStore();
    await store.load();
    store.setCollapsed(work.id, true);

    const made = await store.addGroup("Shelf", work.id);

    expect(made?.id).toBeTruthy();
    expect(groupSaveMock.mock.calls[0]?.[0]).toMatchObject({ name: "Shelf", parentId: work.id, order: 1 });
    expect(store.collapsed.has(work.id)).toBe(false);
    expect(shape(store)).toContain("  Shelf");
  });

  it("keeps a group write failure as an envelope and adds nothing to the list", async () => {
    groupSaveMock.mockRejectedValue(refusal("STORE-IO"));

    const store = useSessionsStore();
    const made = await store.addGroup("Lab");

    expect(made).toBeNull();
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
    // The session is still here, and now it sits at the root — the invariant the confirmation in
    // the sidebar says out loud.
    expect(shape(store)).toEqual(["Lab", "web"]);
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
    const child: SessionGroup = { id: "group-3", name: "Inside", order: 0, parentId: work.id, pinned: false };
    groupsListMock.mockResolvedValue([work, child]);

    const store = useSessionsStore();
    await store.load();
    await store.removeGroup(work.id);

    expect(store.groups.map((group) => [group.id, group.parentId])).toEqual([["group-3", undefined]]);
    expect(shape(store)).toEqual(["Inside"]);
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
    expect(shape(store)).toEqual(["Lab", "Work"]);
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

  it("pins a group with one write, and does not touch the order it was arranged into", async () => {
    groupsListMock.mockResolvedValue([work, lab]);

    const store = useSessionsStore();
    await store.load();
    await store.setGroupPinned(lab.id, true);

    // The whole group goes back, because a group is saved whole — and everything in it is what it
    // already was apart from the one field.
    expect(groupSaveMock.mock.calls.map(([group]) => group)).toEqual([{ ...lab, pinned: true }]);
    expect(shape(store)).toEqual(["Lab", "Work"]);
  });

  it("draws an unpinned group back in its own place, which is what the pin was laid over", async () => {
    groupsListMock.mockResolvedValue([{ ...lab, pinned: true }, work]);

    const store = useSessionsStore();
    await store.load();
    expect(shape(store)).toEqual(["Lab", "Work"]);

    await store.setGroupPinned(lab.id, false);

    expect(shape(store)).toEqual(["Work", "Lab"]);
    expect(groupSaveMock.mock.calls.map(([group]) => group)).toEqual([lab]);
  });

  it("writes nothing when the pin is already what was asked for", async () => {
    groupsListMock.mockResolvedValue([work]);

    const store = useSessionsStore();
    await store.load();
    const worked = await store.setGroupPinned(work.id, false);

    expect(worked).toBe(true);
    expect(groupSaveMock).not.toHaveBeenCalled();
  });

  it("writes nothing for a group that is not there", async () => {
    const store = useSessionsStore();

    expect(await store.setGroupPinned("group-that-is-gone", true)).toBe(false);
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
    expect(shape(store)).toEqual(["Lab", "  Work"]);
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
    expect(shape(store)).toEqual(["Work", "  web"]);
  });

  it("moves a session out of every group when it is sent to the root", async () => {
    sessionsListMock.mockResolvedValue([{ ...draft, groupId: work.id }]);
    groupsListMock.mockResolvedValue([work]);
    sessionSaveMock.mockImplementation(async (session: Session) => session);

    const store = useSessionsStore();
    await store.load();
    await store.placeSession(draft.id, { kind: "root" });

    expect(sessionSaveMock.mock.calls[0]?.[0]?.groupId).toBeUndefined();
    expect(shape(store)).toEqual(["Work", "web"]);
  });

  it("opens the group a session was dropped into, so it does not vanish into a closed group", async () => {
    sessionsListMock.mockResolvedValue([draft]);
    groupsListMock.mockResolvedValue([work]);

    const store = useSessionsStore();
    await store.load();
    store.setCollapsed(work.id, true);
    await store.placeSession(draft.id, { kind: "group", id: work.id });

    expect(store.collapsed.has(work.id)).toBe(false);
    expect(shape(store)).toEqual(["Work", "  web"]);
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

describe("narrowing the list", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    sessionsListMock.mockReset().mockResolvedValue([]);
    groupsListMock.mockReset().mockResolvedValue([]);
    sessionSaveMock.mockReset().mockImplementation(async (session: Session) => session);
  });

  it("narrows the rows without touching the library itself", async () => {
    sessionsListMock.mockResolvedValue([draft, { ...draft, id: "session-2", name: "db", groupId: work.id }]);
    groupsListMock.mockResolvedValue([work]);

    const store = useSessionsStore();
    await store.load();
    store.setQuery("db");

    expect(shape(store)).toEqual(["Work", "  db"]);
    // The filter is a view: what is saved, and what a write will act on, is still everything.
    expect(store.sessions.map((session) => session.name)).toEqual(["web", "db"]);
    expect(store.groups.map((group) => group.id)).toEqual([work.id]);
  });

  it("draws a folder open when a match is inside it, whatever was closed", async () => {
    sessionsListMock.mockResolvedValue([{ ...draft, id: "session-2", name: "db", groupId: work.id }]);
    groupsListMock.mockResolvedValue([work]);

    const store = useSessionsStore();
    await store.load();
    store.setCollapsed(work.id, true);
    expect(shape(store)).toEqual(["Work"]);

    store.setQuery("db");
    expect(shape(store)).toEqual(["Work", "  db"]);

    store.clearFilter();
    // Closing a group is not undone by having searched inside it.
    expect(shape(store)).toEqual(["Work"]);
  });

  it("keeps the sessions that authenticate the way that was chosen, and takes it back", async () => {
    sessionsListMock.mockResolvedValue([
      draft,
      { ...draft, id: "session-2", name: "keyed", auth: { method: "publicKey", hasSecret: false } },
    ]);

    const store = useSessionsStore();
    await store.load();

    store.toggleMethod("publicKey");
    expect(store.filtering).toBe(true);
    expect(shape(store)).toEqual(["keyed"]);

    store.toggleMethod("publicKey");
    expect(store.filtering).toBe(false);
    expect(shape(store)).toEqual(["web", "keyed"]);
  });

  it("will not turn on while nothing is open, because it would show nothing at all", async () => {
    sessionsListMock.mockResolvedValue([draft, { ...draft, id: "session-2", name: "db" }]);

    const store = useSessionsStore();
    await store.load();

    store.toggleOnlyOpen();

    // A press that does nothing: the list is unchanged and the switch is not left looking pressed.
    expect(store.filter.onlyOpen).toBe(false);
    expect(store.filtering).toBe(false);
    expect(shape(store)).toEqual(["web", "db"]);
  });

  it("turns itself off when the last open session goes away", async () => {
    sessionsListMock.mockResolvedValue([draft]);
    const terminals = useTerminalsStore();
    terminals.tabs = [
      { kind: "terminal", id: "tab-1", handle: "h1", session: draft, status: "open", failure: null, panel: null },
    ];

    const store = useSessionsStore();
    await store.load();
    store.toggleOnlyOpen();
    expect(store.filter.onlyOpen).toBe(true);
    expect(shape(store)).toEqual(["web"]);

    terminals.tabs = [];

    expect(store.filter.onlyOpen, "the filter belongs to what is open").toBe(false);
    expect(shape(store)).toEqual(["web"]);
  });

  it("clears both halves of the narrowing at once", async () => {
    sessionsListMock.mockResolvedValue([draft, { ...draft, id: "session-2", name: "keyed" }]);

    const store = useSessionsStore();
    await store.load();
    store.setQuery("keyed");
    store.toggleMethod("password");

    store.clearFilter();

    expect(store.filtering).toBe(false);
    expect(store.filter.query).toBe("");
    expect(shape(store)).toEqual(["web", "keyed"]);
  });

  it("still moves a session into a group the search is hiding", async () => {
    // Which is the whole reason the filter is a view rather than a filtered copy: what a write acts
    // on is the library, not the list.
    sessionsListMock.mockResolvedValue([draft]);
    groupsListMock.mockResolvedValue([work, lab]);

    const store = useSessionsStore();
    await store.load();
    store.setQuery("web");
    expect(shape(store)).toEqual(["web"]);

    const worked = await store.placeSession(draft.id, { kind: "group", id: lab.id });

    expect(worked).toBe(true);
    expect(sessionSaveMock.mock.calls[0]?.[0]).toMatchObject({ id: "session-1", groupId: "group-2" });
  });
});

/** The receipt an import that worked comes back with. */
const imported = (secrets: number): SessionImportOutcome => ({
  kind: "imported",
  summary: { sessions: 1, groups: 0, secrets, missingKeys: 0, droppedHops: 0 },
});

const sealed: SessionImportOutcome = { kind: "needsPassphrase", fileName: "library.json" };

describe("importing a sealed file", () => {
  beforeEach(() => {
    sessionsExportMock.mockReset();
    sessionsImportMock.mockReset();
  });

  it("stops on the question, and finishes it when the passphrase arrives", async () => {
    sessionsImportMock.mockResolvedValueOnce(sealed).mockResolvedValueOnce(imported(1));
    const store = useSessionsStore();

    const running = store.importLibrary("Import sessions");

    // Parked: the call has not come back, and what it is waiting for is on screen.
    await vi.waitFor(() => expect(store.importQuestion).not.toBeNull());
    expect(store.importQuestion).toMatchObject({ fileName: "library.json", busy: false, failure: null });

    store.answerImportPassphrase("correct horse");

    await expect(running).resolves.toMatchObject({ sessions: 1, secrets: 1 });
    expect(store.importQuestion).toBeNull();
    expect(sessionsImportMock.mock.calls[1]).toEqual(["Import sessions", "correct horse"]);
  });

  it("keeps the question open and says why when the passphrase is refused", async () => {
    sessionsImportMock
      .mockResolvedValueOnce(sealed)
      .mockRejectedValueOnce(refusal("SESSION-BUNDLE-PASSPHRASE"))
      .mockResolvedValueOnce(imported(1));
    const store = useSessionsStore();

    const running = store.importLibrary("Import sessions");
    await vi.waitFor(() => expect(store.importQuestion).not.toBeNull());

    store.answerImportPassphrase("rong");

    // The same question, still open, now with the reason in it — not a closed dialog and an error
    // somewhere else. The field the person just filled in is where they should read it.
    await vi.waitFor(() => expect(store.importQuestion?.failure?.code).toBe("SESSION-BUNDLE-PASSPHRASE"));
    expect(store.importQuestion?.busy).toBe(false);
    expect(store.failure).toBeNull();

    store.answerImportPassphrase("correct horse");

    await expect(running).resolves.toMatchObject({ secrets: 1 });
    expect(store.importQuestion).toBeNull();
    expect(sessionsImportMock).toHaveBeenCalledTimes(3);
  });

  it("gives up when the question is dismissed, and imports nothing", async () => {
    sessionsImportMock.mockResolvedValueOnce(sealed);
    const store = useSessionsStore();

    const running = store.importLibrary("Import sessions");
    await vi.waitFor(() => expect(store.importQuestion).not.toBeNull());

    store.answerImportPassphrase(null);

    await expect(running).resolves.toBeNull();
    expect(store.importQuestion).toBeNull();
    expect(sessionsImportMock).toHaveBeenCalledTimes(1);
  });

  it("reports a file that cannot be read where the import's other failures go", async () => {
    sessionsImportMock.mockResolvedValueOnce(sealed).mockRejectedValueOnce(refusal("SESSION-BUNDLE-INVALID"));
    const store = useSessionsStore();

    const running = store.importLibrary("Import sessions");
    await vi.waitFor(() => expect(store.importQuestion).not.toBeNull());

    store.answerImportPassphrase("correct horse");

    await expect(running).resolves.toBeNull();
    expect(store.failure?.code).toBe("SESSION-BUNDLE-INVALID");
    expect(store.importQuestion).toBeNull();
  });

  it("imports a plain file without asking anything", async () => {
    sessionsImportMock.mockResolvedValueOnce(imported(0));
    const store = useSessionsStore();

    await expect(store.importLibrary("Import sessions")).resolves.toMatchObject({ sessions: 1, secrets: 0 });

    expect(store.importQuestion).toBeNull();
    expect(sessionsImportMock.mock.calls[0]).toEqual(["Import sessions", null]);
  });
});

describe("exporting the library", () => {
  beforeEach(() => {
    sessionsExportMock.mockReset();
  });

  it("hands the shell the passphrase, which is the whole of what this side decides", async () => {
    sessionsExportMock.mockResolvedValue({
      sessions: 2,
      groups: 1,
      secrets: 2,
      encrypted: true,
      fileName: "7shell-sessions.json",
    });
    const store = useSessionsStore();

    const summary = await store.exportLibrary("Export sessions", "7shell-sessions.json", "correct horse");

    expect(summary?.encrypted).toBe(true);
    expect(sessionsExportMock.mock.calls[0]).toEqual(["Export sessions", "7shell-sessions.json", "correct horse"]);
  });

  it("says nothing travelled when the answer was no passphrase", async () => {
    sessionsExportMock.mockResolvedValue({
      sessions: 2,
      groups: 1,
      secrets: 0,
      encrypted: false,
      fileName: "7shell-sessions.json",
    });
    const store = useSessionsStore();

    const summary = await store.exportLibrary("Export sessions", "7shell-sessions.json", null);

    expect(summary?.secrets).toBe(0);
    expect(summary?.encrypted).toBe(false);
    expect(sessionsExportMock.mock.calls[0]?.[2]).toBeNull();
  });
});
