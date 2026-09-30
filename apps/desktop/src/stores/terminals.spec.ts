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
  tunnelsOpen: vi.fn(),
  tunnelClose: vi.fn(),
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

import {
  knownHostForget,
  knownHostTrust,
  sessionOpen,
  terminalClose,
  tunnelClose,
  tunnelsOpen,
} from "@/lib/backend/tauri";
import { endedTerminal, onTerminalEnded } from "@/lib/terminal/bus";
import { tunnelFailureKey, useTerminalsStore } from "@/stores/terminals";
import type { OpenTunnel, PresentedHostKey, Session, TunnelFailure } from "@/types/session";

const sessionOpenMock = vi.mocked(sessionOpen);
const terminalCloseMock = vi.mocked(terminalClose);
const knownHostTrustMock = vi.mocked(knownHostTrust);
const knownHostForgetMock = vi.mocked(knownHostForget);
const tunnelsOpenMock = vi.mocked(tunnelsOpen);
const tunnelCloseMock = vi.mocked(tunnelClose);
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

const tunnel: OpenTunnel = {
  id: "handle-1-forward-1",
  handle: "handle-1",
  sessionId: session.id,
  bindHost: "127.0.0.1",
  bindPort: 18080,
  targetHost: "db.internal",
  targetPort: 5432,
  connections: 3,
};

const otherTunnel: OpenTunnel = { ...tunnel, id: "handle-1-forward-2", bindPort: 18081, connections: 0 };

const failure: TunnelFailure = {
  forwardId: "forward-2",
  bindHost: "127.0.0.1",
  bindPort: 18081,
  detail: "the forward could not listen on 127.0.0.1:18081: address already in use",
};

const otherFailure: TunnelFailure = { ...failure, forwardId: "forward-3", bindPort: 18082 };

/** An opened answer, with the two lists every real one carries. */
function opened(tunnels: OpenTunnel[] = [], tunnelFailures: TunnelFailure[] = []) {
  return { kind: "opened", handle: "handle-1", sessionId: session.id, tunnels, tunnelFailures } as const;
}

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
    tunnelsOpenMock.mockReset().mockResolvedValue([]);
    tunnelCloseMock.mockReset().mockResolvedValue(undefined);
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
    sessionOpenMock.mockResolvedValueOnce({ kind: "hostKeyUnknown", key: presented }).mockResolvedValueOnce(opened());

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
      .mockResolvedValueOnce(opened());

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
    sessionOpenMock.mockResolvedValue(opened());

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
    sessionOpenMock.mockResolvedValue(opened());

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
    sessionOpenMock.mockResolvedValue(opened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    const id = store.tabs[0]?.id ?? "";
    await store.closeTab(id);

    expect(terminalCloseMock).toHaveBeenCalledWith("handle-1");
    expect(store.tabs).toHaveLength(0);
  });

  it("records the forwards an open answer brought up, and the ones that did not come up", async () => {
    sessionOpenMock.mockResolvedValue(opened([tunnel], [failure]));

    const store = useTerminalsStore();
    open();
    await store.open(session);

    expect(store.tunnels).toEqual([tunnel]);
    expect(store.tunnelFailures).toEqual([failure]);
  });

  it("dismisses one failed forward without touching the others", async () => {
    sessionOpenMock.mockResolvedValue(opened([], [failure, otherFailure]));

    const store = useTerminalsStore();
    open();
    await store.open(session);
    store.dismissTunnelFailure(tunnelFailureKey(failure));

    expect(store.tunnelFailures).toEqual([otherFailure]);
  });

  it("stops a forward and drops it from the list", async () => {
    sessionOpenMock.mockResolvedValue(opened([tunnel, otherTunnel]));

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.stopTunnel(tunnel.id);

    expect(tunnelCloseMock).toHaveBeenCalledWith(tunnel.id);
    expect(store.tunnels).toEqual([otherTunnel]);
  });

  it("keeps the row and records the problem when a forward refuses to stop", async () => {
    sessionOpenMock.mockResolvedValue(opened([tunnel]));
    tunnelCloseMock.mockRejectedValue({
      version: 1,
      code: "APP-INTERNAL",
      messageKey: "errors.app.internal",
      messageParams: { operation: "tunnel_close" },
      origin: { subsystem: "app" },
      outcome: "unknown",
    });

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.stopTunnel(tunnel.id);

    expect(store.tunnels).toEqual([tunnel]);
    expect(store.tunnelProblem?.code).toBe("APP-INTERNAL");
  });

  it("re-reads the forwards when a terminal is closed, because its forwards go with it", async () => {
    sessionOpenMock.mockResolvedValue(opened([tunnel]));

    const store = useTerminalsStore();
    open();
    await store.open(session);
    expect(store.tunnels).toEqual([tunnel]);

    tunnelsOpenMock.mockResolvedValue([]);
    await store.closeTab(store.tabs[0]?.id ?? "");

    expect(tunnelsOpenMock).toHaveBeenCalled();
    expect(store.tunnels).toEqual([]);
  });
});
