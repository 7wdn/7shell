import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: () => true }));
vi.mock("@/lib/backend/tauri", () => ({
  terminalWrite: vi.fn(async () => {}),
  terminalWriteText: vi.fn(async () => {}),
  terminalClose: vi.fn(async () => {}),
  tunnelsOpen: vi.fn(async () => []),
}));

import { terminalWrite, terminalWriteText } from "@/lib/backend/tauri";
import { decodeBase64, registerInputTracker, stopTerminalBus } from "@/lib/terminal/bus";
import { type TerminalTab, useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

const options = { encoding: "utf-8", backspace: "del" } as const;
const session: Session = {
  id: "local",
  name: "Local",
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

function pane(id: string): TerminalTab {
  return {
    kind: "terminal",
    id,
    session: { ...session, id, name: id },
    handle: `h-${id}`,
    status: "open",
    failure: null,
    panel: null,
  };
}

function textOfWrites(): string[][] {
  return vi
    .mocked(terminalWrite)
    .mock.calls.map(([handle, data]) => [handle, new TextDecoder().decode(decodeBase64(data))]);
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  const store = useTerminalsStore();
  store.tabs = [pane("a"), pane("b"), pane("c"), pane("outside")];
  store.activeTabId = "a";
  store.movePaneIntoSplit("b", "a", "right");
  store.movePaneIntoSplit("c", "b", "down");
  store.activatePane("a");
});

afterEach(() => stopTerminalBus());

describe("terminal input broadcasting", () => {
  it("starts off and sends ordinary input to precisely its source", async () => {
    const store = useTerminalsStore();
    expect(store.broadcast).toBeNull();
    await store.sendInteractive("h-a", "a\r", options);
    expect(textOfWrites()).toEqual([["h-a", "a\r"]]);
  });

  it("sends to selected panes in this workspace and keeps an excluded source independent", async () => {
    const store = useTerminalsStore();
    store.prepareBroadcast();
    store.setBroadcastPane("c", false);
    expect(store.startBroadcast()).toBe(true);
    await store.sendInteractive("h-a", "selected\r", options);
    expect(textOfWrites()).toEqual([
      ["h-a", "selected\r"],
      ["h-b", "selected\r"],
    ]);
    vi.mocked(terminalWrite).mockClear();
    await store.sendInteractive("h-c", "only-c\r", options);
    expect(textOfWrites()).toEqual([["h-c", "only-c\r"]]);
  });

  it("keeps terminal parser replies on their original connection while broadcasting", async () => {
    const store = useTerminalsStore();
    store.startBroadcast();
    await store.send("h-a", new TextEncoder().encode("\x1b[8;24;80t"));
    expect(textOfWrites()).toEqual([["h-a", "\x1b[8;24;80t"]]);
  });

  it("requires two running targets and stops as soon as the selection is edited", () => {
    const store = useTerminalsStore();
    store.startBroadcast();
    store.setBroadcastPane("b", false);
    expect(store.broadcast?.enabled).toBe(false);
    store.setBroadcastPane("c", false);
    expect(store.startBroadcast()).toBe(false);
    expect(store.inputHandles("h-a")).toEqual(["h-a"]);
  });

  it("stops when a target reconnects and never enrolls its replacement handle automatically", () => {
    const store = useTerminalsStore();
    store.startBroadcast();
    const b = store.terminalTabs.find((tab) => tab.id === "b")!;
    b.handle = "h-reconnected";
    expect(store.broadcast?.enabled).toBe(false);
    expect(store.inputHandles("h-a")).toEqual(["h-a"]);
  });

  it("stops on a closed or detached target and on a different workspace", async () => {
    const store = useTerminalsStore();
    store.startBroadcast();
    await store.closePane("b");
    expect(store.broadcast?.enabled).toBe(false);
    expect(store.broadcast?.paneIds).not.toContain("b");
    store.startBroadcast();
    store.detachPane("c");
    expect(store.broadcast).toBeNull();
    store.activatePane("a");
    store.startBroadcast();
    store.activatePane("outside");
    expect(store.broadcast).toBeNull();
  });

  it("uses each target's encoding and backspace setting", async () => {
    const store = useTerminalsStore();
    const b = store.terminalTabs.find((tab) => tab.id === "b")!;
    store.tabs = store.tabs.map((tab) =>
      tab.id === "b"
        ? {
            ...b,
            session: {
              ...session,
              protocol: "ssh",
              terminal: { ...session.terminal, encoding: "gbk", backspace: "ctrlH" },
            },
          }
        : tab,
    );
    store.startBroadcast();
    await store.sendInteractive("h-a", "中文\x7f", options);
    expect(terminalWriteText).toHaveBeenCalledExactlyOnceWith("h-b", "中文\x08", "gbk");
    expect(textOfWrites()).toEqual([
      ["h-a", "中文\x7f"],
      ["h-c", "中文\x7f"],
    ]);
  });

  it("updates peer input tracking and avoids recording a secret if any target is answering", async () => {
    const store = useTerminalsStore();
    const received = vi.fn();
    registerInputTracker("h-a", { input: received, answering: () => false });
    registerInputTracker("h-b", { input: received, answering: () => true });
    store.startBroadcast();
    await store.sendInteractive("h-a", "answer\r", options);
    expect(received).toHaveBeenCalledTimes(2);
    expect(received).toHaveBeenNthCalledWith(1, "answer\r", false);
    expect(received).toHaveBeenNthCalledWith(2, "answer\r", false);
  });

  it("serializes writes per target and snapshots the selection for queued input", async () => {
    const store = useTerminalsStore();
    let finish!: () => void;
    vi.mocked(terminalWrite).mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    store.startBroadcast();
    const first = store.sendInteractive("h-a", "first", options);
    const second = store.sendInteractive("h-a", "second", options);
    store.setBroadcastPane("b", false);
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    expect(textOfWrites().filter(([handle]) => handle === "h-a")).toEqual([["h-a", "first"]]);
    finish();
    await Promise.all([first, second]);
    for (const handle of ["h-a", "h-b", "h-c"]) {
      expect(textOfWrites().filter(([target]) => target === handle)).toEqual([
        [handle, "first"],
        [handle, "second"],
      ]);
    }
  });

  it("stops broadcasting after a failed write while delivering the current input to healthy peers", async () => {
    const store = useTerminalsStore();
    vi.mocked(terminalWrite).mockRejectedValueOnce({
      code: "SSH-CHANNEL-FAILED",
      messageKey: "errors.ssh.channelFailed",
      origin: { subsystem: "ssh" },
      outcome: "unknown",
    });
    store.startBroadcast();
    await store.sendInteractive("h-a", "line\r", options);
    expect(textOfWrites()).toHaveLength(3);
    expect(store.broadcast?.enabled).toBe(false);
    expect(store.terminalTabs.find((tab) => tab.id === "a")?.failure?.code).toBe("SSH-CHANNEL-FAILED");
  });

  it("orders a command operation between earlier and later keyboard input", async () => {
    const store = useTerminalsStore();
    let finishKey!: () => void;
    let finishCommand!: () => void;
    vi.mocked(terminalWrite).mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finishKey = resolve;
        }),
    );
    const before = store.sendInteractive("h-a", "before", options);
    const run = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          finishCommand = resolve;
        }),
    );
    const command = store.withInputOrder(["h-a"], run);
    const after = store.sendInteractive("h-a", "after", options);
    await vi.waitFor(() => expect(finishKey).toBeTypeOf("function"));
    expect(run).not.toHaveBeenCalled();
    finishKey();
    await vi.waitFor(() => expect(run).toHaveBeenCalledOnce());
    expect(textOfWrites()).toEqual([["h-a", "before"]]);
    finishCommand();
    await Promise.all([before, command, after]);
    expect(textOfWrites()).toEqual([
      ["h-a", "before"],
      ["h-a", "after"],
    ]);
  });
});
