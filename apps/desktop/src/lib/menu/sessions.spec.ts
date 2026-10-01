/**
 * What a saved session and a running forward offer.
 *
 * Both menus are the sidebar's own buttons plus the address, which is the thing somebody actually
 * wants out of a session list — and the address is printed exactly the way the row prints it, so what
 * is copied is what was read.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText } from "./clipboard";
import { sessionEndpoint, sessionMenu, tunnelMenu } from "./sessions";
import type { MenuAction, MenuEntry } from "./types";
import type { OpenTunnel, Session } from "@/types/session";

vi.mock("./clipboard", () => ({ copyText: vi.fn(async () => true) }));

const session: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 2222,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30 },
  updatedAt: 1_700_000_000,
};

const tunnel: OpenTunnel = {
  id: "tunnel-1",
  handle: "t1",
  sessionId: session.id,
  bindHost: "127.0.0.1",
  bindPort: 8080,
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

function sessionActions() {
  return { openTerminal: vi.fn(), openFiles: vi.fn(), edit: vi.fn(), remove: vi.fn() };
}

beforeEach(() => {
  vi.mocked(copyText).mockClear();
});

describe("a saved session's menu", () => {
  it("opens both halves, edits, deletes and copies the address", () => {
    const entries = sessionMenu(session, sessionActions());

    expect(entries.filter((entry) => entry.kind === "action").map((entry) => entry.id)).toEqual([
      "open-terminal",
      "open-files",
      "edit",
      "remove",
      "copy-endpoint",
    ]);
    expect(pick(entries, "remove").danger).toBe(true);
  });

  it("names the session in the two entries that open something", () => {
    const entries = sessionMenu(session, sessionActions());

    expect(pick(entries, "open-terminal").params).toEqual({ name: "web" });
    expect(pick(entries, "open-files").params).toEqual({ name: "web" });
  });

  it("hands each entry the session it belongs to", () => {
    const wired = sessionActions();
    const entries = sessionMenu(session, wired);

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

    pick(sessionMenu(session, sessionActions()), "copy-endpoint").run();

    expect(copyText).toHaveBeenCalledWith("root@example.test:2222");
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
