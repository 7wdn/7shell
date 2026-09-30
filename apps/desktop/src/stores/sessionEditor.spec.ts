import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionsList: vi.fn(),
  sessionSave: vi.fn(),
  sessionDelete: vi.fn(),
  groupsList: vi.fn(),
}));

import { groupsList, sessionSave, sessionsList } from "@/lib/backend/tauri";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import type { Session } from "@/types/session";

const sessionsListMock = vi.mocked(sessionsList);
const sessionSaveMock = vi.mocked(sessionSave);
const groupsListMock = vi.mocked(groupsList);

function sample(id: string, name: string): Session {
  return {
    id,
    name,
    protocol: "ssh",
    host: `${name}.test`,
    port: 22,
    user: "root",
    auth: { method: "password", hasSecret: false },
    terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30 },
    updatedAt: 1_700_000_000,
  };
}

describe("session editor store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    sessionsListMock.mockReset().mockResolvedValue([sample("bastion", "bastion"), sample("db", "db")]);
    groupsListMock.mockReset().mockResolvedValue([]);
    sessionSaveMock.mockReset().mockImplementation(async (session: Session) => session);
  });

  it("keeps hops in the order they were added, moved and removed", async () => {
    await useSessionsStore().load();
    const editor = useSessionEditorStore();
    editor.fill(null);

    editor.addHop("bastion");
    editor.addHop("db");
    editor.addHop("bastion");
    expect(editor.draft().jump).toEqual(["bastion", "db", "bastion"]);

    // A move off either end is not a move.
    editor.moveHop(0, -1);
    expect(editor.draft().jump).toEqual(["bastion", "db", "bastion"]);

    editor.moveHop(2, -1);
    expect(editor.draft().jump).toEqual(["bastion", "bastion", "db"]);

    editor.removeHopAt(0);
    expect(editor.draft().jump).toEqual(["bastion", "db"]);
  });

  it("never offers the session being edited as its own hop", async () => {
    await useSessionsStore().load();
    const editor = useSessionEditorStore();
    editor.fill(sample("db", "db"));

    expect(editor.hopChoices.map((session) => session.id)).toEqual(["bastion"]);
    editor.addHop("db");

    expect(editor.jumps).toEqual([]);
  });

  it("keeps a hop whose session was deleted, shows it as missing, and does not save until it is removed", async () => {
    await useSessionsStore().load();
    const editor = useSessionEditorStore();
    editor.fill({ ...sample("app", "app"), jump: ["session-that-is-gone", "bastion"] });

    expect(editor.hops).toHaveLength(2);
    expect(editor.hops[0]).toEqual({ id: "session-that-is-gone", session: undefined });
    expect(editor.hops[1]?.session?.name).toBe("bastion");
    // Still saved: dropping it silently would change the route without telling anyone.
    expect(editor.draft().jump).toEqual(["session-that-is-gone", "bastion"]);

    editor.removeHopAt(0);
    expect(editor.draft().jump).toEqual(["bastion"]);
  });

  it("starts a forward row at the local default and rejects one without a target", async () => {
    const editor = useSessionEditorStore();
    // A complete session, so the only thing under test is the row.
    editor.fill(sample("app", "app"));
    editor.addForward();

    const id = editor.forwards[0]?.id ?? "";
    expect(editor.forwards[0]).toMatchObject({ bindHost: "127.0.0.1", bindPort: "0", targetHost: "", targetPort: "" });
    expect(editor.forwardProblems[id]).toEqual(["targetHost", "targetPort"]);
    expect(editor.hasProblems).toBe(true);

    const row = editor.forwards[0];
    if (!row) {
      throw new Error("addForward should have added a row");
    }

    row.bindPort = "70000";
    expect(editor.forwardProblems[id]).toEqual(["bindPort", "targetHost", "targetPort"]);

    row.bindPort = "0";
    row.targetHost = "db.internal";
    row.targetPort = "5432";
    expect(editor.forwardProblems[id]).toEqual([]);
    expect(editor.hasProblems).toBe(false);
  });

  it("saves a valid forward row as part of the session", async () => {
    const library = useSessionsStore();
    const editor = useSessionEditorStore();
    editor.fill(sample("app", "app"));
    editor.addForward();

    const row = editor.forwards[0];
    if (!row) {
      throw new Error("addForward should have added a row");
    }
    row.targetHost = "db.internal";
    row.targetPort = "5432";

    await library.save(editor.draft());

    expect(sessionSaveMock).toHaveBeenCalledWith(
      expect.objectContaining({
        forwards: [{ id: row.id, bindHost: "127.0.0.1", bindPort: 0, targetHost: "db.internal", targetPort: 5432 }],
      }),
      undefined,
    );
    expect(library.sessions[0]?.forwards?.[0]?.targetPort).toBe(5432);
  });

  it("saves no route or forwards at all when there are none", async () => {
    const library = useSessionsStore();
    const editor = useSessionEditorStore();
    editor.fill(sample("app", "app"));

    const saved = editor.draft();
    expect(saved.jump).toBeUndefined();
    expect(saved.forwards).toBeUndefined();

    await library.save(saved);
    expect(sessionSaveMock).toHaveBeenCalledWith(saved, undefined);
  });
});
