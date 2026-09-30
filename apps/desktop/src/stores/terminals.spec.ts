import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionOpen: vi.fn(),
  terminalClose: vi.fn(),
  terminalWrite: vi.fn(),
  terminalResize: vi.fn(),
  knownHostTrust: vi.fn(),
  knownHostForget: vi.fn(),
}));
// The bus is mocked rather than run: these tests are about what the store does with an event,
// and the bus already has its own suite for delivering one.
vi.mock("@/lib/terminal/bus", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@/lib/terminal/bus")>();
  return {
    ...actual,
    startTerminalBus: vi.fn(async () => {}),
    onTerminalEnded: vi.fn(() => () => {}),
    endedTerminal: vi.fn(() => undefined),
  };
});

import { knownHostForget, knownHostTrust, sessionOpen, terminalClose } from "@/lib/backend/tauri";
import { endedTerminal, onTerminalEnded } from "@/lib/terminal/bus";
import { useTerminalsStore } from "@/stores/terminals";
import type { PresentedHostKey, Session } from "@/types/session";

const sessionOpenMock = vi.mocked(sessionOpen);
const terminalCloseMock = vi.mocked(terminalClose);
const knownHostTrustMock = vi.mocked(knownHostTrust);
const knownHostForgetMock = vi.mocked(knownHostForget);
const onTerminalEndedMock = vi.mocked(onTerminalEnded);
const endedTerminalMock = vi.mocked(endedTerminal);

const session: Session = {
  id: "session-1",
  name: "web",
  protocol: "ssh",
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password", hasSecret: true },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30 },
  updatedAt: 1_700_000_000,
};

const presented: PresentedHostKey = {
  host: "example.test",
  port: 22,
  keyType: "ssh-ed25519",
  publicKey: "AAAAB3NzaC1lZDI1NTE5AAAAIexample",
  fingerprint: "SHA256:presented",
};

const trusted = { ...presented, fingerprint: "SHA256:trusted", addedAt: 1_690_000_000 };

/** The listener the store registered, so a test can end a terminal the way the bus would. */
const endedListeners: ((handle: string, reason?: string) => void)[] = [];

function open(): void {
  useTerminalsStore().initialize();
}

describe("terminals store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    endedListeners.length = 0;
    sessionOpenMock.mockReset();
    terminalCloseMock.mockReset().mockResolvedValue(undefined);
    knownHostTrustMock.mockReset().mockResolvedValue(undefined);
    knownHostForgetMock.mockReset().mockResolvedValue(undefined);
    onTerminalEndedMock.mockReset().mockImplementation((listener: (handle: string, reason?: string) => void) => {
      endedListeners.push(listener);
      return () => {};
    });
    endedTerminalMock.mockReset().mockReturnValue(undefined);
  });

  it("creates no tab when the host key has to be decided first", async () => {
    sessionOpenMock.mockResolvedValue({ kind: "hostKeyUnknown", key: presented });

    const store = useTerminalsStore();
    open();
    await store.open(session);

    // Nothing was opened, so there is no terminal to draw — only a question to answer.
    expect(store.tabs).toHaveLength(0);
    expect(store.decision).toEqual({ kind: "hostKeyUnknown", session, key: presented });
  });

  it("trusts the key and opens again, with the tab appearing after the second answer", async () => {
    sessionOpenMock
      .mockResolvedValueOnce({ kind: "hostKeyUnknown", key: presented })
      .mockResolvedValueOnce({ kind: "opened", handle: "handle-1", sessionId: session.id });

    const store = useTerminalsStore();
    open();
    await store.open(session);
    expect(store.tabs).toHaveLength(0);

    await store.confirmTrust();

    expect(knownHostTrustMock).toHaveBeenCalledWith(presented);
    expect(sessionOpenMock).toHaveBeenCalledTimes(2);
    expect(store.decision).toBeNull();
    expect(store.tabs).toHaveLength(1);
    expect(store.tabs[0]?.handle).toBe("handle-1");
    expect(store.tabs[0]?.status).toBe("open");
  });

  it("forgets the old key before trusting the new one on a mismatch", async () => {
    sessionOpenMock
      .mockResolvedValueOnce({ kind: "hostKeyMismatch", key: presented, trusted })
      .mockResolvedValueOnce({ kind: "opened", handle: "handle-2", sessionId: session.id });

    const store = useTerminalsStore();
    open();
    await store.open(session);
    expect(store.decision?.kind).toBe("hostKeyMismatch");

    await store.confirmReplace();

    expect(knownHostForgetMock).toHaveBeenCalledWith("example.test", 22);
    expect(knownHostTrustMock).toHaveBeenCalledWith(presented);
    expect(knownHostForgetMock.mock.invocationCallOrder[0]).toBeLessThan(
      knownHostTrustMock.mock.invocationCallOrder[0] ?? 0,
    );
    expect(store.decision).toBeNull();
  });

  it("answers cancel by leaving nothing pending", async () => {
    sessionOpenMock.mockResolvedValue({ kind: "hostKeyUnknown", key: presented });

    const store = useTerminalsStore();
    open();
    await store.open(session);
    store.dismiss();

    expect(store.decision).toBeNull();
    expect(store.tabs).toHaveLength(0);
    expect(knownHostTrustMock).not.toHaveBeenCalled();
  });

  it("keeps the tab, marked ended with the reason, when the terminal closes", async () => {
    sessionOpenMock.mockResolvedValue({ kind: "opened", handle: "handle-1", sessionId: session.id });

    const store = useTerminalsStore();
    open();
    await store.open(session);

    for (const listener of endedListeners) {
      listener("handle-1", "connection closed by remote host");
    }

    expect(store.tabs).toHaveLength(1);
    expect(store.tabs[0]?.status).toBe("ended");
    expect(store.tabs[0]?.reason).toBe("connection closed by remote host");
  });

  it("shows a terminal that died before its tab existed as already ended", async () => {
    endedTerminalMock.mockReturnValue({ reason: "the server refused the channel" });
    sessionOpenMock.mockResolvedValue({ kind: "opened", handle: "handle-1", sessionId: session.id });

    const store = useTerminalsStore();
    open();
    await store.open(session);

    expect(store.tabs[0]?.status).toBe("ended");
    expect(store.tabs[0]?.reason).toBe("the server refused the channel");
  });

  it("gives a failed open a tab, because that is where the person is looking", async () => {
    sessionOpenMock.mockRejectedValue({
      version: 1,
      code: "SSH-AUTH-FAILED",
      messageKey: "errors.ssh.authFailed",
      messageParams: { host: "example.test", port: "22" },
      origin: { subsystem: "ssh" },
      outcome: "sent",
    });

    const store = useTerminalsStore();
    open();
    await store.open(session);

    expect(store.tabs).toHaveLength(1);
    expect(store.tabs[0]?.status).toBe("failed");
    expect(store.tabs[0]?.failure?.code).toBe("SSH-AUTH-FAILED");
    // Nothing was opened, so there is no handle to close later.
    expect(store.tabs[0]?.handle).toBeNull();
  });

  it("removes a tab whose terminal is closed", async () => {
    sessionOpenMock.mockResolvedValue({ kind: "opened", handle: "handle-1", sessionId: session.id });

    const store = useTerminalsStore();
    open();
    await store.open(session);
    const id = store.tabs[0]?.id ?? "";
    await store.closeTab(id);

    expect(terminalCloseMock).toHaveBeenCalledWith("handle-1");
    expect(store.tabs).toHaveLength(0);
  });
});
