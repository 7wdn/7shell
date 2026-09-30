import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionsList: vi.fn(),
  sessionSave: vi.fn(),
  sessionDelete: vi.fn(),
  groupsList: vi.fn(),
}));

import { groupsList, sessionDelete, sessionSave, sessionsList } from "@/lib/backend/tauri";
import { useSessionsStore } from "@/stores/sessions";
import type { Session } from "@/types/session";

const sessionsListMock = vi.mocked(sessionsList);
const sessionSaveMock = vi.mocked(sessionSave);
const sessionDeleteMock = vi.mocked(sessionDelete);
const groupsListMock = vi.mocked(groupsList);

const draft: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: false },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30 },
  updatedAt: 1_700_000_000,
};

describe("sessions store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    sessionsListMock.mockReset().mockResolvedValue([]);
    groupsListMock.mockReset().mockResolvedValue([]);
    sessionSaveMock.mockReset().mockImplementation(async (session: Session) => session);
    sessionDeleteMock.mockReset().mockResolvedValue(undefined);
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
    sessionDeleteMock.mockRejectedValue({
      version: 1,
      code: "STORE-IO",
      messageKey: "errors.store.io",
      messageParams: {},
      origin: { subsystem: "store" },
      outcome: "rejected",
    });

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

    expect(store.sections.map((section) => section.name)).toEqual(["Work", null]);
    expect(store.sections[0]?.sessions.map((session) => session.id)).toEqual(["session-1"]);
    expect(store.sections[1]?.sessions.map((session) => session.id)).toEqual(["session-2", "session-3"]);
  });
});
