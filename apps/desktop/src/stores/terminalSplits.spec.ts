import { createPinia, setActivePinia } from "pinia";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: () => true }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionOpen: vi.fn(),
  sessionSystem: vi.fn(),
  terminalClose: vi.fn(async () => {}),
  terminalWrite: vi.fn(async () => {}),
  tunnelsOpen: vi.fn(async () => []),
  filesClose: vi.fn(async () => {}),
  knownHostTrust: vi.fn(async () => {}),
}));

import { knownHostTrust, sessionOpen, terminalClose, terminalWrite } from "@/lib/backend/tauri";
import { decodeBase64 } from "@/lib/terminal/bus";
import { terminalPaneIds } from "@/lib/workspace/terminal-layout";
import { usePreferencesStore } from "@/stores/preferences";
import { useFilesStore } from "@/stores/files";
import { type TerminalTab, useTerminalsStore } from "@/stores/terminals";
import type { PresentedHostKey, Session } from "@/types/session";

const session: Session = {
  id: "local-a",
  name: "Local A",
  protocol: "local",
  host: "",
  port: 0,
  user: "",
  auth: { method: "password", hasSecret: false },
  terminal: {
    terminalType: "xterm-256color",
    keepaliveSeconds: 30,
    encoding: "utf-8",
    backspace: "del",
    forwardAgent: false,
    record: false,
  },
  updatedAt: 1,
};
const other: Session = { ...session, id: "local-b", name: "Local B" };
let nextHandle = 0;

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  nextHandle = 0;
  usePreferencesStore().sessionLayout = "terminal";
  vi.mocked(sessionOpen).mockImplementation(async (sessionId) => ({
    kind: "opened",
    handle: `h-${++nextHandle}`,
    sessionId,
    tunnels: [],
    tunnelFailures: [],
  }));
});

async function pair(): Promise<{ first: TerminalTab; second: TerminalTab; workspaceId: string }> {
  const store = useTerminalsStore();
  await store.open(session);
  const first = store.terminalTabs[0] as TerminalTab;
  await store.splitNewTerminal(first.id, "right", other);
  return { first, second: store.terminalTabs[1] as TerminalTab, workspaceId: store.activeWorkspaceId as string };
}

describe("terminal split ownership", () => {
  it("keeps an unsaved pane when its workspace closes and closes the other panes normally", async () => {
    const store = useTerminalsStore();
    const { first, second, workspaceId } = await pair();
    second.panel = { handle: "files-b", status: "open", failure: null };
    useFilesStore().setUnsaved("files-b:/tmp/draft.txt", true);
    await store.closeTab(workspaceId);
    expect(store.terminalTabs).toEqual([second]);
    expect(terminalClose).toHaveBeenCalledExactlyOnceWith(first.handle);
    expect(second.panel?.handle).toBe("files-b");
  });
  it("releases a late connection when its destination pane was closed during authentication", async () => {
    const store = useTerminalsStore();
    await store.open(session);
    const first = store.terminalTabs[0] as TerminalTab;
    let finish!: (outcome: Awaited<ReturnType<typeof sessionOpen>>) => void;
    vi.mocked(sessionOpen).mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    const splitting = store.splitNewTerminal(first.id, "right", other);
    await store.closePane(first.id);
    finish({ kind: "opened", handle: "h-late", sessionId: other.id, tunnels: [], tunnelFailures: [] });
    await splitting;
    expect(store.tabs).toHaveLength(0);
    expect(terminalClose).toHaveBeenLastCalledWith("h-late");
    expect(store.connectingSessionIds).toEqual([]);
  });
  it("opens independent handles in one workspace and writes only to its focused terminal", async () => {
    const store = useTerminalsStore();
    const { first, second } = await pair();
    expect(store.workspaceTabs).toHaveLength(1);
    expect(store.openSessionCount).toBe(2);
    expect(store.activeTerminalHandle()).toBe(second.handle);
    expect(first.handle).not.toBe(second.handle);
    store.activatePane(first.id);
    await store.send(store.activeTerminalHandle() as string, new TextEncoder().encode("first\r"));
    expect(vi.mocked(terminalWrite).mock.calls).toHaveLength(1);
    expect(vi.mocked(terminalWrite).mock.calls[0]?.[0]).toBe(first.handle);
    expect(new TextDecoder().decode(decodeBase64(vi.mocked(terminalWrite).mock.calls[0]?.[1] ?? ""))).toBe("first\r");
  });

  it("moves a running terminal with its panel instead of reconnecting or closing it", async () => {
    const store = useTerminalsStore();
    await store.open(session);
    await store.open(other);
    const first = store.terminalTabs[0] as TerminalTab;
    const second = store.terminalTabs[1] as TerminalTab;
    second.panel = { handle: "files-b", status: "open", failure: null };
    const panel = second.panel;
    expect(store.movePaneIntoSplit(second.id, first.id, "down")).toBe(true);
    expect(store.workspaceTabs).toHaveLength(1);
    expect(store.terminalTabs[1]).toBe(second);
    expect(second.panel).toBe(panel);
    expect(sessionOpen).toHaveBeenCalledTimes(2);
    expect(terminalClose).not.toHaveBeenCalled();
  });

  it("closes the original pane while preserving the workspace and its remaining handle", async () => {
    const store = useTerminalsStore();
    const { first, second, workspaceId } = await pair();
    await store.closePane(first.id);
    expect(store.workspaceTabs.map((tab) => tab.id)).toEqual([workspaceId]);
    expect(store.terminalTabs).toEqual([second]);
    expect(store.activeTerminalHandle()).toBe(second.handle);
    expect(terminalClose).toHaveBeenCalledExactlyOnceWith(first.handle);
    await store.closeTab(workspaceId);
    expect(store.tabs).toHaveLength(0);
    expect(store.workspaceTabs).toHaveLength(0);
    expect(terminalClose).toHaveBeenLastCalledWith(second.handle);
  });

  it("closes only the selected session in a workspace containing different hosts", async () => {
    const store = useTerminalsStore();
    const { first, second } = await pair();
    await store.closeSessions([first.session.id]);
    expect(store.terminalTabs).toEqual([second]);
    expect(terminalClose).toHaveBeenCalledExactlyOnceWith(first.handle);
  });

  it("keeps all panes of the selected workspace when closing other tabs", async () => {
    const store = useTerminalsStore();
    const { workspaceId } = await pair();
    await store.open({ ...session, id: "local-c" });
    await store.closeOthers(workspaceId);
    expect(store.terminalTabs).toHaveLength(2);
    expect(store.workspaceTabs).toHaveLength(1);
    expect(terminalClose).toHaveBeenCalledExactlyOnceWith("h-3");
  });

  it("detaches the original pane into another tab without an id collision or a new connection", async () => {
    const store = useTerminalsStore();
    const { first, second, workspaceId } = await pair();
    store.detachPane(first.id);
    expect(store.workspaceTabs).toHaveLength(2);
    expect(new Set(store.workspaceTabs.map((tab) => tab.id)).size).toBe(2);
    expect(store.activeWorkspaceId).not.toBe(workspaceId);
    expect(store.activeTerminalHandle()).toBe(first.handle);
    store.activate(workspaceId);
    expect(store.activeTerminalHandle()).toBe(second.handle);
    expect(sessionOpen).toHaveBeenCalledTimes(2);
    await store.closeAll();
    expect(store.tabs).toHaveLength(0);
    expect(terminalClose).toHaveBeenCalledTimes(2);
  });

  it("moves a pane inside a nested workspace without duplicating its handle", async () => {
    const store = useTerminalsStore();
    const { first, second } = await pair();
    await store.splitNewTerminal(second.id, "down");
    const third = store.terminalTabs[2] as TerminalTab;
    store.movePaneIntoSplit(third.id, first.id, "down");
    expect(terminalPaneIds(store.terminalWorkspaces[0]!.layout)).toEqual([first.id, third.id, second.id]);
    expect(new Set(store.writableHandles()).size).toBe(3);
    expect(sessionOpen).toHaveBeenCalledTimes(3);
  });

  it("remembers the focused pane across tab switches and preserves the tree while zoomed", async () => {
    const store = useTerminalsStore();
    const { first, second, workspaceId } = await pair();
    const layout = store.terminalWorkspaces[0]!.layout;
    store.togglePaneZoom(first.id);
    store.focusNextPane(1);
    expect(store.activeTerminalHandle()).toBe(second.handle);
    expect(store.terminalWorkspaces[0]?.zoomedId).toBe(second.id);
    store.togglePaneZoom(second.id);
    expect(store.terminalWorkspaces[0]?.zoomedId).toBeNull();
    expect(store.terminalWorkspaces[0]?.layout).toBe(layout);
    store.openSettings();
    store.activate(workspaceId);
    expect(store.activeTerminalHandle()).toBe(second.handle);
  });

  it("returns a host key decision to the chosen split without creating an empty pane", async () => {
    const store = useTerminalsStore();
    await store.open(session);
    const first = store.terminalTabs[0] as TerminalTab;
    const key: PresentedHostKey = {
      host: "lab.test",
      port: 22,
      keyType: "ssh-ed25519",
      publicKey: "public",
      fingerprint: "SHA256:test",
    };
    vi.mocked(sessionOpen).mockResolvedValueOnce({ kind: "hostKeyUnknown", key });
    await store.splitNewTerminal(first.id, "down", { ...other, protocol: "ssh", host: key.host });
    expect(store.terminalTabs).toHaveLength(1);
    expect(store.decision?.target).toMatchObject({ to: "terminal", split: { paneId: first.id, direction: "down" } });
    await store.confirmTrust();
    expect(knownHostTrust).toHaveBeenCalledExactlyOnceWith(key);
    expect(store.terminalTabs).toHaveLength(2);
    expect(store.workspaceTabs).toHaveLength(1);
    expect(store.terminalWorkspaces[0]?.layout).toMatchObject({ direction: "down" });
  });
});
