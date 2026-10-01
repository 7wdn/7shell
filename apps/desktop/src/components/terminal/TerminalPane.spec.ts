/**
 * The wiring between a terminal and the file panel under it.
 *
 * The rules have their own suite, and so do the stores; what this proves is that they are attached
 * to each other — that what somebody types reaches the rules, that the shell's own reports do, and
 * that a `cd` moves the browser. That is the whole feature, and it is four lines in a component
 * that no other test would look at.
 *
 * xterm is replaced rather than driven. A real one needs a laid-out document to exist in at all,
 * and the question here is not how xterm reads a key press — it is what this pane does with what it
 * read.
 */
import { createApp, defineComponent, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** What xterm handed the pane, so a test can type into it the way xterm would. */
let onData: ((data: string) => void) | undefined;
let oscHandlers: Map<number, (data: string) => boolean>;

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    readonly parser = {
      registerOscHandler: (identifier: number, handler: (data: string) => boolean) => {
        oscHandlers.set(identifier, handler);
        return { dispose: () => oscHandlers.delete(identifier) };
      },
    };
    options = {};
    cols = 80;
    rows = 24;

    loadAddon(): void {}
    open(): void {}
    write(): void {}
    focus(): void {}
    dispose(): void {}
    onData(handler: (data: string) => void): { dispose: () => void } {
      onData = handler;
      return { dispose: () => {} };
    }
  },
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit(): void {}
  },
}));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
// The real bus, with only the one call the pane makes replaced: the stores need the rest of it to
// open a terminal at all, and a thinner mock would fail the open rather than the test.
vi.mock("@/lib/terminal/bus", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/terminal/bus")>()),
  registerTerminalSink: vi.fn(() => () => {}),
}));
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

import { createPinia, setActivePinia } from "pinia";

import TerminalPane from "@/components/terminal/TerminalPane.vue";
import { filesList, filesOpen, sessionOpen, transfersOpen, tunnelsOpen } from "@/lib/backend/tauri";
import { useFilesStore } from "@/stores/files";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

const sessionOpenMock = vi.mocked(sessionOpen);
const filesOpenMock = vi.mocked(filesOpen);
const filesListMock = vi.mocked(filesList);
const tunnelsOpenMock = vi.mocked(tunnelsOpen);
const transfersOpenMock = vi.mocked(transfersOpen);

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

/** The session opened with a terminal and its panel, which is the state this pane is tested in. */
async function withPanelUnderTerminal(): Promise<void> {
  sessionOpenMock.mockResolvedValue({
    kind: "opened",
    handle: "handle-1",
    sessionId: session.id,
    tunnels: [],
    tunnelFailures: [],
  });
  filesOpenMock.mockResolvedValue({
    kind: "opened",
    handle: "f1",
    sessionId: session.id,
    listing: { path: "/home/user", entries: [] },
  });
  filesListMock.mockImplementation(async (_handle: string, path: string) => ({ path, entries: [] }));

  const terminals = useTerminalsStore();
  await terminals.open(session);
  const tab = terminals.tabs.find((entry) => entry.kind === "terminal");
  if (!tab) {
    throw new Error("the terminal did not open");
  }
  await terminals.togglePanel(tab);
}

let unmount: (() => void) | undefined;

/** Mounts one pane for the open terminal, which is what registers the handlers under test. */
async function mountPane(): Promise<void> {
  const host = defineComponent({
    setup: () => () => h(TerminalPane, { handle: "handle-1", active: true, interactive: true }),
  });
  const app = createApp(host);
  const root = document.createElement("div");
  document.body.append(root);
  app.mount(root);
  await nextTick();

  // A pane that registered nothing would make every case below pass by doing nothing at all.
  if (!onData || !oscHandlers.has(7)) {
    throw new Error("the pane registered no handlers with its terminal");
  }

  unmount = () => {
    app.unmount();
    root.remove();
  };
}

describe("a terminal carrying a file panel", () => {
  beforeEach(async () => {
    setActivePinia(createPinia());
    // The panel is opened here by hand rather than by the setting, so that what is being tested is
    // the pane and not the default.
    usePreferencesStore().sessionLayout = "terminal";
    onData = undefined;
    oscHandlers = new Map();
    vi.stubGlobal(
      "ResizeObserver",
      class {
        observe(): void {}
        unobserve(): void {}
        disconnect(): void {}
      },
    );
    sessionOpenMock.mockReset();
    filesOpenMock.mockReset();
    filesListMock.mockReset();
    tunnelsOpenMock.mockReset().mockResolvedValue([]);
    transfersOpenMock.mockReset().mockResolvedValue([]);

    await withPanelUnderTerminal();
    await mountPane();
  });

  afterEach(() => {
    unmount?.();
    unmount = undefined;
    document.body.innerHTML = "";
  });

  it("takes the panel to the directory a typed cd names", async () => {
    onData?.("cd /data\r");
    await nextTick();
    await nextTick();

    expect(filesListMock).toHaveBeenLastCalledWith("f1", "/data");
    expect(useFilesStore().browser("f1")?.path).toBe("/data");
  });

  it("takes the panel to the directory the shell reports, exactly", async () => {
    oscHandlers.get(7)?.("file://web/var/log");
    await nextTick();
    await nextTick();

    expect(useFilesStore().browser("f1")?.path).toBe("/var/log");
  });

  it("leaves the panel where it is for a line that names no directory", async () => {
    onData?.("systemctl restart nginx\r");
    await nextTick();
    await nextTick();

    expect(filesListMock).not.toHaveBeenCalled();
    expect(useFilesStore().browser("f1")?.path).toBe("/home/user");
  });

  it("reads the line across several keystrokes, not one call at a time", async () => {
    onData?.("cd /va");
    onData?.("r/log");
    onData?.("\r");
    await nextTick();
    await nextTick();

    expect(useFilesStore().browser("f1")?.path).toBe("/var/log");
  });
});
