import { createPinia, setActivePinia } from "pinia";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: () => true }));
vi.mock("@/lib/backend/tauri", () => ({
  commandSend: vi.fn(),
  commandHistory: vi.fn(async () => []),
  snippetsOpen: vi.fn(async () => []),
  terminalWrite: vi.fn(async () => {}),
}));

import { commandSend } from "@/lib/backend/tauri";
import { registerInputReset, registerInputTracker, stopTerminalBus } from "@/lib/terminal/bus";
import { useCommandsStore } from "@/stores/commands";
import { useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

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

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  const terminals = useTerminalsStore();
  terminals.tabs = ["a", "b", "c"].map((id) => ({
    kind: "terminal",
    id,
    session,
    handle: `h-${id}`,
    status: "open",
    failure: null,
    panel: null,
  }));
  terminals.activeTabId = "a";
  terminals.movePaneIntoSplit("b", "a", "right");
  terminals.startBroadcast();
  vi.mocked(commandSend).mockResolvedValue({ kind: "sent", report: { sent: ["h-a", "h-b"], failures: [] } });
});

afterEach(() => stopTerminalBus());

describe("command input broadcast targets", () => {
  it("uses the selected workspace handles and sends through the existing risk gate", async () => {
    await useCommandsStore().sendInputLine("h-a", "echo shared");
    expect(commandSend).toHaveBeenCalledExactlyOnceWith(
      { kind: "terminals", handles: ["h-a", "h-b"] },
      "echo shared",
      true,
      false,
    );
  });

  it("returns a required approval without authorizing the command", async () => {
    const outcome = { kind: "approvalRequired", risk: { level: "destructive", reasons: [] } } as const;
    vi.mocked(commandSend).mockResolvedValue(outcome);
    expect(await useCommandsStore().sendInputLine("h-a", "rm -rf /tmp/test")).toEqual(outcome);
    expect(commandSend).toHaveBeenCalledTimes(1);
    expect(vi.mocked(commandSend).mock.calls[0]?.[3]).toBe(false);
  });

  it("binds approval to the exact set of targets and sends nothing after they change", async () => {
    const terminals = useTerminalsStore();
    const approved = terminals.inputHandles("h-a");
    terminals.setBroadcastPane("b", false);
    expect(await useCommandsStore().sendInputLine("h-a", "rm -rf /tmp/test", approved)).toBeUndefined();
    expect(commandSend).not.toHaveBeenCalled();
  });

  it("forwards acknowledgement only for the targets a person reviewed", async () => {
    const handles = useTerminalsStore().inputHandles("h-a");
    await useCommandsStore().sendInputLine("h-a", "echo reviewed", handles);
    expect(commandSend).toHaveBeenCalledExactlyOnceWith({ kind: "terminals", handles }, "echo reviewed", true, true);
  });

  it("keeps a file panel's explicit directory command on its own terminal", async () => {
    await useCommandsStore().sendLineTo("h-a", "cd /tmp");
    expect(commandSend).toHaveBeenCalledExactlyOnceWith(
      { kind: "terminals", handles: ["h-a"] },
      "cd /tmp",
      true,
      false,
    );
  });

  it("stops broadcasting when a command reaches only some of its targets", async () => {
    vi.mocked(commandSend).mockResolvedValue({
      kind: "sent",
      report: {
        sent: ["h-a"],
        failures: [
          { handle: "h-b", code: "SESSION-NOT-FOUND", messageKey: "errors.session.notFound", messageParams: {} },
        ],
      },
    });
    await useCommandsStore().sendInputLine("h-a", "echo partial");
    expect(useTerminalsStore().broadcast?.enabled).toBe(false);
    expect(useCommandsStore().sendFailures[0]?.handle).toBe("h-b");
  });

  it("preserves input tracking for keys queued behind a command", async () => {
    const tracked = ["old line"];
    registerInputTracker("h-a", { input: (data) => tracked.push(data), answering: () => false });
    registerInputReset("h-a", () => {
      tracked.length = 0;
    });
    let finish!: () => void;
    vi.mocked(commandSend).mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = () => resolve({ kind: "sent", report: { sent: ["h-a", "h-b"], failures: [] } });
        }),
    );
    const command = useCommandsStore().sendInputLine("h-a", "echo sent");
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    const key = useTerminalsStore().sendInteractive("h-a", "next", { encoding: "utf-8", backspace: "del" });
    expect(tracked).toEqual(["old line"]);
    finish();
    await Promise.all([command, key]);
    expect(tracked).toEqual(["next"]);
  });
});
