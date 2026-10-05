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

import { i18n } from "@/i18n";

/** What xterm handed the pane, so a test can type into it the way xterm would. */
let onData: ((data: string) => void) | undefined;
/** The pane's key handler, so a test can ask whether the terminal keeps Ctrl+F for itself. */
let keyHandler: ((event: KeyboardEvent) => boolean) | undefined;
let oscHandlers: Map<number, (data: string) => boolean>;

/** What the pane asked of the terminal it owns, so a menu entry can be watched doing it. */
let selection = "";
let pasted: string[] = [];
let selectedAll = 0;
let cleared = 0;

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
    /** Kept, so a test can press Ctrl+F the way xterm would route it. */
    attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean): void {
      keyHandler = handler;
    }
    search(): void {}
    open(): void {}
    write(): void {}
    focus(): void {}
    dispose(): void {}
    onData(handler: (data: string) => void): { dispose: () => void } {
      onData = handler;
      return { dispose: () => {} };
    }

    getSelection(): string {
      return selection;
    }

    paste(text: string): void {
      pasted.push(text);
    }

    selectAll(): void {
      selectedAll += 1;
    }

    clear(): void {
      cleared += 1;
    }
  },
}));
vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class {
    fit(): void {}
  },
}));
vi.mock("@xterm/addon-web-links", () => ({ WebLinksAddon: class {} }));
vi.mock("@xterm/addon-search", () => ({
  SearchAddon: class {
    onDidChangeResults(): void {}
    findNext(): void {}
    findPrevious(): void {}
    clearDecorations(): void {}
  },
}));
// The real bus, with only the one call the pane makes replaced: the stores need the rest of it to
// open a terminal at all, and a thinner mock would fail the open rather than the test.
vi.mock("@/lib/terminal/bus", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/terminal/bus")>()),
  registerTerminalSink: vi.fn(() => () => {}),
}));
vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  clipboardText: vi.fn(),
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
import { useContextMenu } from "@/composables/useContextMenu";
import { clipboardText, filesList, filesOpen, sessionOpen, transfersOpen, tunnelsOpen } from "@/lib/backend/tauri";
import type { MenuAction } from "@/lib/menu/types";
import { useFilesStore } from "@/stores/files";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

const sessionOpenMock = vi.mocked(sessionOpen);
const filesOpenMock = vi.mocked(filesOpen);
const filesListMock = vi.mocked(filesList);
const tunnelsOpenMock = vi.mocked(tunnelsOpen);
const transfersOpenMock = vi.mocked(transfersOpen);
/** The clipboard as Rust reads it, which is the only way this build can read one at all. */
const clipboardTextMock = vi.mocked(clipboardText);

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
/** The element the pane drew itself into, which is what a right-click lands on. */
let pane: HTMLElement | undefined;

/** Mounts one pane for the open terminal, which is what registers the handlers under test. */
async function mountPane(interactive = true): Promise<void> {
  const host = defineComponent({
    setup: () => () => h(TerminalPane, { handle: "handle-1", active: true, interactive }),
  });
  // The real translations rather than stubs: the pane now draws a find bar whose labels are keys,
  // and a pane mounted without them throws before any of these tests get to look at it.
  const app = createApp(host).use(i18n);
  const root = document.createElement("div");
  document.body.append(root);
  app.mount(root);
  await nextTick();

  // A pane that registered nothing would make every case below pass by doing nothing at all.
  if (!onData || !oscHandlers.has(7)) {
    throw new Error("the pane registered no handlers with its terminal");
  }

  pane = root.querySelector<HTMLElement>(".terminal-surface") ?? undefined;
  unmount = () => {
    app.unmount();
    root.remove();
  };
}

// One pane, opened the same way for both halves of this file: a terminal with its panel under it,
// which is the state the pane is drawn in when somebody is working at it.
beforeEach(async () => {
  setActivePinia(createPinia());
  // The panel is opened here by hand rather than by the setting, so that what is being tested is
  // the pane and not the default.
  usePreferencesStore().sessionLayout = "terminal";
  onData = undefined;
  oscHandlers = new Map();
  selection = "";
  pasted = [];
  selectedAll = 0;
  cleared = 0;
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
  clipboardTextMock.mockReset().mockResolvedValue("");

  await withPanelUnderTerminal();
  await mountPane();
});

afterEach(() => {
  unmount?.();
  unmount = undefined;
  document.body.innerHTML = "";
  vi.unstubAllGlobals();
});

describe("a terminal carrying a file panel", () => {
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

/**
 * The terminal's right-click, which is what the file panel's tests above have nothing to do with: it
 * is about the terminal half of the same tab.
 *
 * The two halves of every case are the hand-over — the pane builds the list and the shell's layer
 * would draw it — and what the entries do to the terminal they came from.
 */
describe("a terminal's right-click menu", () => {
  /** Opens the menu the way a right-click does, and hands back the event it was opened with. */
  function rightClick(): MouseEvent {
    const event = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 10, clientY: 10 });
    pane?.dispatchEvent(event);
    return event;
  }

  function action(id: string): MenuAction {
    const found = useContextMenu().entries.value.find(
      (entry): entry is MenuAction => entry.kind === "action" && entry.id === id,
    );
    if (!found) {
      throw new Error(`the menu has no entry called ${id}`);
    }
    return found;
  }

  it("offers the terminal a menu of its own, and cancels the browser's", () => {
    selection = "systemctl status nginx";

    const event = rightClick();

    const { visible, entries } = useContextMenu();
    expect(visible.value).toBe(true);
    expect(event.defaultPrevented).toBe(true);
    expect(entries.value.map((entry) => entry.id)).toEqual(["copy", "paste", "select-all", "sep-screen", "clear"]);
    expect(action("copy").disabled).toBeFalsy();
  });

  it("offers no copy when nothing is selected", () => {
    rightClick();

    expect(action("copy").disabled).toBe(true);
  });

  it("pastes the clipboard through the terminal rather than around it", async () => {
    clipboardTextMock.mockResolvedValue("ls -la\n");
    rightClick();

    await action("paste").run();

    // Through xterm's own paste, which is what makes a bracketed or multi-line paste behave the way
    // the shell expects; writing the bytes directly would skip all of that.
    expect(clipboardTextMock).toHaveBeenCalledOnce();
    expect(pasted).toEqual(["ls -la\n"]);
  });

  it("pastes nothing when the clipboard holds no text", async () => {
    clipboardTextMock.mockResolvedValue("");
    rightClick();

    await action("paste").run();

    expect(pasted).toEqual([]);
  });

  it("offers no paste into a terminal that has ended", async () => {
    unmount?.();
    document.body.innerHTML = "";
    await mountPane(false);

    rightClick();

    expect(action("paste").disabled).toBe(true);
  });

  it("selects everything and clears the screen through the terminal", () => {
    rightClick();

    action("select-all").run();
    action("clear").run();

    expect(selectedAll).toBe(1);
    expect(cleared).toBe(1);
  });
});

describe("the keys the terminal keeps for itself", () => {
  beforeEach(async () => {
    await mountPane();
  });

  it("keeps Ctrl+F for the find bar instead of sending it to the shell", () => {
    const kept = keyHandler?.({ type: "keydown", ctrlKey: true, metaKey: false, key: "f" } as KeyboardEvent);

    // `false` is xterm's "do not handle this": without it the shell receives ^F, which on some
    // hosts is a real control character and not a search.
    expect(kept).toBe(false);
  });

  it("passes everything else through", () => {
    expect(keyHandler?.({ type: "keydown", ctrlKey: false, metaKey: false, key: "c" } as KeyboardEvent)).toBe(true);
    expect(keyHandler?.({ type: "keydown", ctrlKey: true, metaKey: false, key: "d" } as KeyboardEvent)).toBe(true);
  });
});
