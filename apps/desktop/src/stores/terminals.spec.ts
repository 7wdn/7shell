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
  filesOpen: vi.fn(),
  filesList: vi.fn(),
  filesClose: vi.fn(),
  transfersOpen: vi.fn(),
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
  filesClose,
  filesList,
  filesOpen,
  knownHostForget,
  knownHostTrust,
  sessionOpen,
  terminalClose,
  transfersOpen,
  tunnelClose,
  tunnelsOpen,
} from "@/lib/backend/tauri";
import { endedTerminal, onTerminalEnded } from "@/lib/terminal/bus";
import { useFilesStore } from "@/stores/files";
import { usePreferencesStore } from "@/stores/preferences";
import { type FilesTab, type TerminalTab, tunnelFailureKey, useTerminalsStore } from "@/stores/terminals";
import type { OpenTunnel, PresentedHostKey, Session, TunnelFailure } from "@/types/session";

const sessionOpenMock = vi.mocked(sessionOpen);
const terminalCloseMock = vi.mocked(terminalClose);
const knownHostTrustMock = vi.mocked(knownHostTrust);
const knownHostForgetMock = vi.mocked(knownHostForget);
const tunnelsOpenMock = vi.mocked(tunnelsOpen);
const tunnelCloseMock = vi.mocked(tunnelClose);
const filesOpenMock = vi.mocked(filesOpen);
const filesListMock = vi.mocked(filesList);
const filesCloseMock = vi.mocked(filesClose);
const transfersOpenMock = vi.mocked(transfersOpen);
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
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
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
  mode: "local",
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

/** A file browser that opened: the browser's own first listing comes with the answer. */
function filesOpened() {
  return {
    kind: "opened",
    handle: "f1",
    sessionId: session.id,
    listing: { path: "/home/user", entries: [] },
  } as const;
}

/** The listener the store registered, so a test can end a terminal the way the bus would. */
const endedListeners: ((handle: string, reason?: string) => void)[] = [];

function open(): void {
  useTerminalsStore().initialize();
}

/** The terminal tab in the workspace, for assertions that are about a terminal rather than a tab. */
function terminalTab(store: ReturnType<typeof useTerminalsStore>): TerminalTab | undefined {
  return store.tabs.find((tab): tab is TerminalTab => tab.kind === "terminal");
}

/** The terminal tab, or a failure: a test with no terminal tab has nothing to assert about one. */
function terminalTabOrFail(store: ReturnType<typeof useTerminalsStore>): TerminalTab {
  const tab = terminalTab(store);
  if (!tab) {
    throw new Error("the workspace has no terminal tab");
  }
  return tab;
}

/** The file browser tab, for the assertions that are about a browser rather than a tab. */
function filesTab(store: ReturnType<typeof useTerminalsStore>): FilesTab | undefined {
  return store.tabs.find((tab): tab is FilesTab => tab.kind === "files");
}

/** The envelope a refused backend call answers with, for the paths that only need a code. */
const refused = {
  version: 1,
  code: "APP-INTERNAL",
  messageKey: "errors.app.internal",
  messageParams: { operation: "files_close" },
  origin: { subsystem: "app" },
  outcome: "unknown",
} as const;

describe("terminals store", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    // How a session opens is a setting, and the default is the split. Most of these cases are about
    // the terminal half alone, so they say which shape they are testing instead of inheriting it.
    usePreferencesStore().sessionLayout = "terminal";
    endedListeners.length = 0;
    sessionOpenMock.mockReset();
    terminalCloseMock.mockReset().mockResolvedValue(undefined);
    knownHostTrustMock.mockReset().mockResolvedValue(undefined);
    knownHostForgetMock.mockReset().mockResolvedValue(undefined);
    tunnelsOpenMock.mockReset().mockResolvedValue([]);
    tunnelCloseMock.mockReset().mockResolvedValue(undefined);
    filesOpenMock.mockReset();
    filesListMock.mockReset().mockResolvedValue({ path: "/home/user", entries: [] });
    filesCloseMock.mockReset().mockResolvedValue(undefined);
    transfersOpenMock.mockReset().mockResolvedValue([]);
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
    expect(store.decision).toEqual({
      kind: "hostKeyUnknown",
      session,
      target: { to: "terminal" },
      key: presented,
    });
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
    expect(terminalTab(store)?.handle).toBe("handle-1");
    expect(terminalTab(store)?.status).toBe("open");
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
    expect(terminalTab(store)?.status).toBe("ended");
    expect(terminalTab(store)?.reason).toBe("connection closed by remote host");
  });

  it("shows a terminal that died before its tab existed as already ended", async () => {
    endedTerminalMock.mockReturnValue({ reason: "the server refused the channel" });
    sessionOpenMock.mockResolvedValue(opened());

    const store = useTerminalsStore();
    open();
    await store.open(session);

    expect(terminalTab(store)?.status).toBe("ended");
    expect(terminalTab(store)?.reason).toBe("the server refused the channel");
  });

  it("keeps a failed open on the row it was made from, and opens nothing", async () => {
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

    // Nothing came up, so there is nothing for the workspace to show: the row the click was made
    // on is where the answer belongs, and it is also the row that can be clicked again.
    expect(store.tabs).toHaveLength(0);
    expect(store.failures[session.id]?.code).toBe("SSH-AUTH-FAILED");
    expect(store.connectingSessionIds).toEqual([]);
  });

  it("forgets a failure when the next attempt works", async () => {
    sessionOpenMock.mockRejectedValueOnce(refused);
    const store = useTerminalsStore();
    open();
    await store.open(session);
    expect(store.failures[session.id]).toBeDefined();

    sessionOpenMock.mockResolvedValue(opened());
    await store.open(session);

    expect(store.failures[session.id]).toBeUndefined();
  });

  it("says an attempt that was called off is not a failure", async () => {
    sessionOpenMock.mockRejectedValue({
      version: 1,
      code: "SESSION-OPEN-CANCELLED",
      messageKey: "errors.session.openCancelled",
      messageParams: {},
      origin: { subsystem: "ssh" },
      outcome: "notSent",
    });

    const store = useTerminalsStore();
    open();
    await store.open(session);

    expect(store.tabs).toHaveLength(0);
    expect(store.failures[session.id], "the person who pressed the cross knows what happened").toBeUndefined();
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

  it("keeps a terminal and a file browser open at once", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.openFiles(session);

    expect(store.tabs.map((tab) => tab.kind)).toEqual(["terminal", "files"]);
    expect(filesTab(store)?.handle).toBe("f1");
    expect(store.activeTabId).toBe(store.tabs[1]?.id);
  });

  it("closes the browser through the backend when its tab is closed", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.openFiles(session);
    await store.closeTab(store.tabs[1]?.id ?? "");

    expect(filesCloseMock).toHaveBeenCalledWith("f1");
    expect(store.tabs.map((tab) => tab.kind)).toEqual(["terminal"]);
  });

  /**
   * The strip's two ways of closing several tabs: every one but this, and every one after this.
   *
   * A page is thrown in because it closes without the backend — a tab holding nothing is still a tab
   * as far as the workspace is concerned, which is exactly the case a loop that only understood
   * terminals would leave behind.
   */
  it("closes every tab but the one it was asked about", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.openFiles(session);
    const terminal = terminalTabOrFail(store);
    store.openInspect("system", session);

    await store.closeOthers(terminal.id);

    expect(filesCloseMock).toHaveBeenCalledWith("f1");
    expect(store.tabs.map((tab) => tab.kind)).toEqual(["terminal"]);
    // The tab that was kept is the one left on screen: the menu was opened on it, so that is where
    // somebody was looking.
    expect(store.activeTabId).toBe(terminal.id);
  });

  it("closes the tabs after one, and leaves the ones before it", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.openFiles(session);
    store.openInspect("system", session);

    await store.closeToTheRight(store.tabs[0]?.id ?? "");

    expect(store.tabs.map((tab) => tab.kind)).toEqual(["terminal"]);

    // And from the middle: what is after it goes, what is before it stays.
    await store.openFiles(session);
    store.openInspect("network", session);
    await store.closeToTheRight(store.tabs[1]?.id ?? "");

    expect(store.tabs.map((tab) => tab.kind)).toEqual(["terminal", "files"]);
  });

  it("keeps the browser tab and says why when the backend refuses to close it", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());
    filesCloseMock.mockRejectedValue(refused);

    const store = useTerminalsStore();
    open();
    await store.openFiles(session);
    await store.closeTab(store.tabs[0]?.id ?? "");

    // The browser is still open on the far side, so the tab that draws it stays.
    expect(store.tabs).toHaveLength(1);
    expect(filesTab(store)?.status).toBe("failed");
  });

  it("focuses the browser it already has instead of opening a second one", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.openFiles(session);
    const filesTab = store.tabs[1]?.id ?? "";
    store.activate(store.tabs[0]?.id ?? "");

    await store.openFiles(session);

    expect(filesOpenMock).toHaveBeenCalledTimes(1);
    expect(store.tabs).toHaveLength(2);
    expect(store.activeTabId).toBe(filesTab);
  });

  it("raises the same host key decision for a file browser, and opens it once answered", async () => {
    filesOpenMock
      .mockResolvedValueOnce({ kind: "hostKeyUnknown", key: presented })
      .mockResolvedValueOnce(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.openFiles(session);

    // A decision is not a browser: there is nothing to draw until it is answered.
    expect(store.tabs).toHaveLength(0);
    expect(store.decision?.kind).toBe("hostKeyUnknown");
    expect(store.decision?.target).toEqual({ to: "files" });

    await store.confirmTrust();

    expect(knownHostTrustMock).toHaveBeenCalledWith(presented);
    expect(filesOpenMock).toHaveBeenCalledTimes(2);
    expect(store.tabs[0]?.kind).toBe("files");
    expect(store.decision).toBeNull();
  });

  it("hangs the file browser under the terminal, in the tab that asked for it", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    const tab = terminalTabOrFail(store);
    await store.togglePanel(tab);

    expect(store.tabs).toHaveLength(1);
    expect(tab.panel).toEqual({ handle: "f1", status: "open", failure: null });
  });

  it("moves the browser a file tab is already drawing into the panel", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.openFiles(session);
    const tab = terminalTabOrFail(store);
    await store.togglePanel(tab);

    // One browser, drawn in one place: two listings of a directory have no way to say which is
    // right, so the one that exists moves rather than a second one being opened beside it.
    expect(filesOpenMock).toHaveBeenCalledTimes(1);
    expect(store.tabs.map((entry) => entry.kind)).toEqual(["terminal"]);
    expect(tab.panel?.handle).toBe("f1");
  });

  it("opens no second browser when the panel is the thing already drawing one", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.togglePanel(terminalTabOrFail(store));
    await store.openFiles(session);

    expect(filesOpenMock).toHaveBeenCalledTimes(1);
    expect(store.tabs.map((entry) => entry.kind)).toEqual(["terminal"]);
  });

  it("closes the browser before the terminal whose connection carries it", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.togglePanel(terminalTabOrFail(store));
    await store.closeTab(store.tabs[0]?.id ?? "");

    expect(filesCloseMock).toHaveBeenCalledWith("f1");
    expect(terminalCloseMock).toHaveBeenCalledWith("handle-1");
    // The SFTP channel rides on that terminal's connection, so the other order would cut it out
    // from under a browser that is still moving files.
    const closedBrowser = filesCloseMock.mock.invocationCallOrder.at(0) ?? 0;
    const closedTerminal = terminalCloseMock.mock.invocationCallOrder.at(0) ?? 0;
    expect(closedBrowser).toBeGreaterThan(0);
    expect(closedBrowser).toBeLessThan(closedTerminal);
    expect(store.tabs).toHaveLength(0);
  });

  it("keeps the tab and says why when the browser in the panel refuses to close", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());
    filesCloseMock.mockRejectedValue(refused);

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.togglePanel(terminalTabOrFail(store));
    await store.closeTab(store.tabs[0]?.id ?? "");

    // The browser is still open on the far side, so the terminal that carries it is not closed.
    expect(terminalCloseMock).not.toHaveBeenCalled();
    expect(store.tabs).toHaveLength(1);
    const tab = terminalTabOrFail(store);
    expect(tab.panel?.status).toBe("failed");
    expect(tab.panel?.handle).toBe("f1");
    expect(tab.panel?.failure?.code).toBe("APP-INTERNAL");
  });

  it("puts a panel that could not be opened in the panel, not in a new tab", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockRejectedValue(refused);

    const store = useTerminalsStore();
    open();
    await store.open(session);
    const tab = terminalTabOrFail(store);
    await store.togglePanel(tab);

    expect(store.tabs).toHaveLength(1);
    expect(tab.panel?.status).toBe("failed");
    expect(tab.panel?.failure?.code).toBe("APP-INTERNAL");
  });

  it("raises the host key decision for a panel and opens it into that tab once answered", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock
      .mockResolvedValueOnce({ kind: "hostKeyUnknown", key: presented })
      .mockResolvedValueOnce(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    const tab = terminalTabOrFail(store);
    await store.togglePanel(tab);

    // A decision is not a browser, and a panel left spinning while it is up would go on spinning
    // if it were cancelled.
    expect(tab.panel).toBeNull();
    expect(store.decision?.target).toEqual({ to: "panel", tabId: tab.id });

    await store.confirmTrust();

    expect(knownHostTrustMock).toHaveBeenCalledWith(presented);
    expect(filesOpenMock).toHaveBeenCalledTimes(2);
    expect(store.tabs).toHaveLength(1);
    expect(terminalTabOrFail(store).panel?.handle).toBe("f1");
  });

  it("opens both halves when the settings say a session opens as two", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());
    usePreferencesStore().sessionLayout = "split";

    const store = useTerminalsStore();
    open();
    await store.open(session);

    expect(store.tabs).toHaveLength(1);
    expect(terminalTabOrFail(store).panel?.handle).toBe("f1");
  });

  it("opens no panel when the terminal never came up, whatever the settings say", async () => {
    sessionOpenMock.mockRejectedValue(refused);
    usePreferencesStore().sessionLayout = "split";

    const store = useTerminalsStore();
    open();
    await store.open(session);

    // There is no tab at all, so there is nothing for a file browser to be opened onto.
    expect(filesOpenMock).not.toHaveBeenCalled();
    expect(store.tabs).toHaveLength(0);
    expect(store.failures[session.id]).toBeDefined();
  });

  it("reconnects in the shape the settings name, not the shape the tab happened to have", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    const tab = terminalTabOrFail(store);
    await store.togglePanel(tab);
    expect(tab.panel?.handle).toBe("f1");

    endedListeners.forEach((listener) => listener("handle-1", "the shell exited"));
    await store.reconnect(tab.id);

    // A reconnect asks for the same thing again, and the settings are what say what that is.
    expect(terminalTabOrFail(store).panel).toBeNull();
  });

  it("leaves the panel standing when the terminal under it ends", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.togglePanel(terminalTabOrFail(store));

    endedListeners.forEach((listener) => listener("handle-1", "the shell exited"));

    // A shell that exited does not take the files with it: what the browser rides on is the
    // connection, and the panel is still the way to them until it is closed.
    expect(terminalTabOrFail(store).status).toBe("ended");
    expect(terminalTabOrFail(store).panel?.handle).toBe("f1");
  });

  it("takes the file panel with the shell when the shell changes directory", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());
    filesListMock.mockResolvedValue({ path: "/data", entries: [] });

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.togglePanel(terminalTabOrFail(store));
    await store.followDirectory("handle-1", "/data");

    expect(filesListMock).toHaveBeenLastCalledWith("f1", "/data");
    expect(useFilesStore().browser("f1")?.path).toBe("/data");
  });

  it("moves nothing when the browser for that session is a tab, not a panel", async () => {
    sessionOpenMock.mockResolvedValue(opened());
    filesOpenMock.mockResolvedValue(filesOpened());

    const store = useTerminalsStore();
    open();
    await store.open(session);
    await store.openFiles(session);
    await store.followDirectory("handle-1", "/data");

    // A file tab is somebody browsing, and their shell moving is not their browsing moving.
    expect(filesListMock).not.toHaveBeenCalled();
  });

  it("moves nothing when the handle belongs to no terminal on screen", async () => {
    const store = useTerminalsStore();
    open();
    await store.followDirectory("handle-gone", "/data");

    expect(filesListMock).not.toHaveBeenCalled();
  });
});

describe("pages about a host", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
    usePreferencesStore().sessionLayout = "terminal";
    tunnelsOpenMock.mockReset().mockResolvedValue([]);
  });

  it("opens a page for a session, and only one page per view of it", () => {
    const store = useTerminalsStore();
    open();

    store.openInspect("system", session);
    store.openInspect("system", session);

    expect(store.tabs).toHaveLength(1);
    expect(store.tabs[0]?.kind).toBe("inspect");
    expect(store.activeTabId).toBe(store.tabs[0]?.id);
  });

  it("gives every view and every host a page of its own", () => {
    const store = useTerminalsStore();
    open();
    const other: Session = { ...session, id: "session-2", name: "db", host: "db.example.test" };

    store.openInspect("system", session);
    store.openInspect("processes", session);
    store.openInspect("system", other);

    expect(store.tabs).toHaveLength(3);
    expect(store.tabs.map((tab) => (tab.kind === "inspect" ? tab.view : tab.kind))).toEqual([
      "system",
      "processes",
      "system",
    ]);
  });

  it("takes somebody to the page they already have rather than opening a second one", () => {
    const store = useTerminalsStore();
    open();
    store.openInspect("system", session);
    store.openInspect("network", session);
    const first = store.tabs[0]?.id ?? "";

    store.openInspect("system", session);

    expect(store.tabs).toHaveLength(2);
    expect(store.activeTabId).toBe(first);
  });

  /** A page holds nothing on the far side, so closing it asks the backend for nothing. */
  it("closes a page without closing anything on the host", async () => {
    const store = useTerminalsStore();
    open();
    store.openInspect("network", session);

    await store.closeTab(store.tabs[0]?.id ?? "");

    expect(store.tabs).toEqual([]);
    expect(terminalCloseMock).not.toHaveBeenCalled();
    expect(filesCloseMock).not.toHaveBeenCalled();
  });

  it("draws no browser for a page, so a dropped file has nowhere to land", () => {
    const store = useTerminalsStore();
    open();
    store.openInspect("processes", session);

    const tab = store.tabs[0];
    expect(tab && store.browserHandleOf(tab)).toBeNull();
  });
});
