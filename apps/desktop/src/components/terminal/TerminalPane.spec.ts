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
import { DEFAULT_TERMINAL_FONT } from "@/types/app";

/** What xterm handed the pane, so a test can type into it the way xterm would. */
let onData: ((data: string) => void) | undefined;
/** The pane's key handler, so a test can ask whether the terminal keeps Ctrl+F for itself. */
let keyHandler: ((event: KeyboardEvent) => boolean) | undefined;
let oscHandlers: Map<number, (data: string) => boolean>;

/** What the pane asked of the terminal it owns, so a menu entry can be watched doing it. */
let selection = "";
/** Every terminal the pane built, so a test can look at what it was handed and when. */
const builtTerminals: { options: Record<string, unknown> }[] = [];
let pasted: string[] = [];
let selectedAll = 0;
let cleared = 0;
/** The handlers the pane registered with the terminal, so a test can make xterm report. */
let selectionChanged: (() => void) | undefined;
let scrolled: (() => void) | undefined;
let droppedSelections = 0;
/** Every search the pane ran, with the options it ran it with. */
let searched: { needle: string; options?: { decorations?: unknown } }[] = [];

vi.mock("@xterm/xterm", () => ({
  Terminal: class {
    options: Record<string, unknown> = {};
    constructor(options: Record<string, unknown> = {}) {
      // What the real one does with the object it is built from, so a test can ask what font the
      // pane opened with rather than only what it was changed to afterwards.
      Object.assign(this.options, options);
      builtTerminals.push(this as unknown as { options: Record<string, unknown> });
    }
    readonly parser = {
      registerOscHandler: (identifier: number, handler: (data: string) => boolean) => {
        oscHandlers.set(identifier, handler);
        return { dispose: () => oscHandlers.delete(identifier) };
      },
    };
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

    hasSelection(): boolean {
      return selection !== "";
    }

    clearSelection(): void {
      droppedSelections += 1;
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

    onSelectionChange(handler: () => void): { dispose: () => void } {
      selectionChanged = handler;
      return { dispose: () => {} };
    }

    onScroll(handler: () => void): { dispose: () => void } {
      scrolled = handler;
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
vi.mock("@xterm/addon-search", () => ({
  SearchAddon: class {
    onDidChangeResults(): void {}
    findNext(needle: string, options?: { decorations?: unknown }): void {
      searched.push({ needle, options });
    }
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
// Only the copy half of the clipboard module is replaced: the paste half is what the cases below
// drive through the backend's own clipboard call, and a wholesale mock would replace the thing
// under test.
vi.mock("@/lib/menu/clipboard", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/menu/clipboard")>()),
  copyText: vi.fn(async () => true),
}));
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
import { copyText } from "@/lib/menu/clipboard";
import type { MenuAction } from "@/lib/menu/types";
import { useAppStore } from "@/stores/app";
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
/** Copying, watched where the entries call it rather than through the platform's clipboard. */
const copyTextMock = vi.mocked(copyText);

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
  if (!onData || !oscHandlers.has(7) || !selectionChanged || !scrolled) {
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
  selectionChanged = undefined;
  scrolled = undefined;
  droppedSelections = 0;
  searched = [];
  copyTextMock.mockClear();
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe(): void {}
      unobserve(): void {}
      disconnect(): void {}
    },
  );
  // The pane reads the renderer on an animation frame, so the frame is made a timer: a test that
  // dispatches a release and then waits a few milliseconds has waited out everything on the other
  // side of it.
  vi.stubGlobal("requestAnimationFrame", (callback: FrameRequestCallback) => window.setTimeout(() => callback(0), 1));
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
    expect(entries.value.map((entry) => entry.id)).toEqual([
      "copy",
      "paste",
      "paste-selection",
      "select-all",
      "sep-screen",
      "clear",
    ]);
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

  it("hands the selection to the terminal itself, without a clipboard in between", async () => {
    selection = "systemctl status nginx";
    rightClick();

    await action("paste-selection").run();

    expect(pasted).toEqual(["systemctl status nginx"]);
    expect(droppedSelections).toBe(1);
    expect(clipboardTextMock).not.toHaveBeenCalled();
  });

  it("selects everything and clears the screen through the terminal", () => {
    rightClick();

    action("select-all").run();
    action("clear").run();

    expect(selectedAll).toBe(1);
    expect(cleared).toBe(1);
  });
});

/**
 * The toolbar over a selection.
 *
 * A test document lays nothing out, so the renderer's selection layer is drawn here by hand — these
 * are the boxes `selectionBox` reads, and where the buttons land in a real window is the end-to-end
 * run's question. What this file can see is the wiring: which gestures raise the toolbar, what the
 * buttons do to the terminal they stand over, and that a terminal with nothing drawn gets no toolbar.
 */
describe("the toolbar over a selection", () => {
  function rect(left: number, top: number, right: number, bottom: number): DOMRect {
    return {
      left,
      top,
      right,
      bottom,
      width: right - left,
      height: bottom - top,
      x: left,
      y: top,
      toJSON: () => ({}),
    } as DOMRect;
  }

  /**
   * The selection as xterm really draws one: the line's own segment, and the band across the row
   * that a one-line selection is given and that has no height of its own.
   *
   * The segment's box is handed back because a test moves it: that is what the renderer does when
   * the terminal scrolls, and following it is the whole of what the toolbar does about a scroll.
   */
  function drawSelection(): { left: number; top: number; right: number; bottom: number } {
    const box = { left: 40, top: 20, right: 160, bottom: 34 };
    const surface = pane as HTMLElement;
    const layer = document.createElement("div");
    layer.className = "xterm-selection";
    const part = document.createElement("div");
    part.getBoundingClientRect = () => rect(box.left, box.top, box.right, box.bottom);
    const band = document.createElement("div");
    band.getBoundingClientRect = () => rect(0, box.bottom, 400, box.bottom);
    layer.append(part, band);
    surface.append(layer);
    surface.getBoundingClientRect = () => rect(0, 0, 400, 300);
    (surface.parentElement as HTMLElement).getBoundingClientRect = () => rect(0, 0, 400, 300);
    return box;
  }

  /** A selection drag: the press, the release, and the frames drawn on either side of them. */
  async function dragSelect(): Promise<void> {
    pane?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 }));
    window.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, button: 0 }));
    await settle();
  }

  /** Waits out the frame the pane reads the renderer on, and the ticks that follow it. */
  async function settle(): Promise<void> {
    await new Promise((resolve) => setTimeout(resolve, 5));
    await nextTick();
    await nextTick();
  }

  const tools = (): HTMLElement | null => document.querySelector('[role="toolbar"]');

  /** One of the toolbar's buttons, by the name it answers to. */
  function toolButton(name: string): HTMLButtonElement {
    const found = tools()?.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`);
    if (!found) {
      throw new Error(`the toolbar has no button called ${name}`);
    }
    return found;
  }

  it("stands over a selection once the drag is released", async () => {
    selection = "systemctl status nginx";
    drawSelection();

    await dragSelect();

    expect(tools()).not.toBeNull();
    const labels = [...(tools()?.querySelectorAll("button") ?? [])].map((button) => button.getAttribute("aria-label"));
    expect(labels).toEqual(["Copy", "Paste selection", "Find in terminal"]);
    // Hung from the end of the selection — the segment stops at 160 — and not from the band across
    // the row, which would put the toolbar at the far edge of the pane instead.
    expect(tools()?.style.left).toBe("148px");
    expect(tools()?.style.top).toBe("40px");
  });

  it("stands away while nothing is selected", async () => {
    selection = "";
    drawSelection();

    await dragSelect();

    expect(tools()).toBeNull();
  });

  it("stands away when the selection goes", async () => {
    selection = "nginx.conf";
    drawSelection();
    await dragSelect();
    expect(tools()).not.toBeNull();

    selection = "";
    selectionChanged?.();
    await nextTick();

    expect(tools()).toBeNull();
  });

  it("stands away when the selection is scrolled out of the drawn rows", async () => {
    selection = "nginx.conf";
    drawSelection();
    await dragSelect();
    expect(tools()).not.toBeNull();

    // What scrolling the selection away leaves behind: the layer redraws without it.
    pane?.querySelector(".xterm-selection")?.remove();
    scrolled?.();
    await settle();

    expect(tools()).toBeNull();
  });

  it("follows the rows when the terminal scrolls under it", async () => {
    selection = "nginx.conf";
    const drawn = drawSelection();
    await dragSelect();
    const before = tools()?.style.top;

    drawn.top = 120;
    drawn.bottom = 134;
    scrolled?.();
    await settle();

    expect(tools()?.style.top).not.toBe(before);
  });

  it("copies what is selected", async () => {
    selection = "docker compose up";
    drawSelection();
    await dragSelect();

    toolButton("Copy").click();
    await nextTick();

    expect(copyTextMock).toHaveBeenCalledWith("docker compose up");
    // Used, and gone: the toolbar is a menu that happens to be drawn as buttons.
    expect(tools()).toBeNull();
  });

  it("hands the selection to the shell without a clipboard in between", async () => {
    selection = "docker compose up";
    drawSelection();
    await dragSelect();

    toolButton("Paste selection").click();
    await nextTick();

    expect(pasted).toEqual(["docker compose up"]);
    expect(droppedSelections).toBe(1);
    expect(clipboardTextMock).not.toHaveBeenCalled();
    expect(tools()).toBeNull();
  });

  it("opens the find bar over the selection", async () => {
    selection = "nginx.conf";
    drawSelection();
    await dragSelect();

    toolButton("Find in terminal").click();
    await nextTick();
    await nextTick();

    const field = document.querySelector<HTMLInputElement>('input[aria-label="Find in terminal"]');
    expect(field, "the bar is up").not.toBeNull();
    expect(field?.value, "and holds what was highlighted").toBe("nginx.conf");
    expect(tools()).toBeNull();
  });

  it("takes the first line that says anything for the find field", async () => {
    // A selection can cover several rows and a find field holds one line, so the needle is the first
    // row with words on it — trimmed, because what somebody highlighted by hand has a margin in it.
    selection = "\n  nginx.conf  \nlisten 80;";
    drawSelection();
    await dragSelect();

    toolButton("Find in terminal").click();
    await nextTick();
    await nextTick();

    const field = document.querySelector<HTMLInputElement>('input[aria-label="Find in terminal"]');
    expect(field?.value).toBe("nginx.conf");
  });

  it("searches what it was handed, asking for the marks the match count is made of", async () => {
    // The count the bar prints comes from the addon's results event, and the addon only reports when
    // it was given colours to draw the matches with — so the request for the marks and the request
    // for the count are one and the same, which is what this pins down.
    selection = "nginx.conf";
    drawSelection();
    await dragSelect();

    toolButton("Find in terminal").click();
    await nextTick();
    await nextTick();

    const last = searched.at(-1);
    expect(last?.needle, "the search was run, over the selection").toBe("nginx.conf");
    expect(last?.options?.decorations, "and with something to draw the matches with").toBeDefined();
  });

  it("keeps paste selection out of a terminal that has ended", async () => {
    unmount?.();
    document.body.innerHTML = "";
    await mountPane(false);
    selection = "old output";
    drawSelection();

    await dragSelect();

    expect(toolButton("Paste selection").disabled).toBe(true);
    expect(toolButton("Copy").disabled, "the scrollback of an ended terminal is still readable").toBe(false);
  });
});

describe("the font the pane draws in", () => {
  /**
   * A terminal is built with its font and does not go looking for another one.
   *
   * So the family has to be handed to the terminal that is already open, which is what makes the
   * setting visible in the pane the person is looking at rather than in the next one they open —
   * and it is also the only way the hover preview can show a font before it is chosen. The grid
   * matters as much as the face: a terminal is a grid of cells as wide as the font it is drawn in,
   * so a new family means a new fit, and the case would pass with the font set and the columns left
   * at the old face's width.
   */
  it("follows a family chosen, or looked at, after the pane was opened", async () => {
    const preferences = usePreferencesStore();
    preferences.initialize();
    await withPanelUnderTerminal();
    await mountPane();

    const terminal = builtTerminals.at(-1);
    expect(terminal?.options.fontFamily, "built with the family the document carried").toBe(DEFAULT_TERMINAL_FONT);

    // What a hover does: the same path, with nothing written down afterwards.
    preferences.previewFont("terminalFont", "'Cascadia Code', monospace");
    await nextTick();
    await nextTick();
    expect(terminal?.options.fontFamily, "a pane that is already open follows").toBe("'Cascadia Code', monospace");

    preferences.clearFontPreview();
    await nextTick();
    await nextTick();
    expect(terminal?.options.fontFamily, "and lets go of it again").toBe(DEFAULT_TERMINAL_FONT);
  });
});

describe("the keys the terminal keeps for itself", () => {
  beforeEach(async () => {
    await mountPane();
  });

  /** A key press carrying every modifier flag, the way a real event does. */
  function key(held: {
    key: string;
    ctrlKey?: boolean;
    metaKey?: boolean;
    shiftKey?: boolean;
    altKey?: boolean;
  }): KeyboardEvent {
    return {
      type: "keydown",
      ctrlKey: false,
      metaKey: false,
      shiftKey: false,
      altKey: false,
      ...held,
    } as KeyboardEvent;
  }

  it("keeps Ctrl+F for the find bar instead of sending it to the shell", () => {
    const kept = keyHandler?.(key({ key: "f", ctrlKey: true }));

    // `false` is xterm's "do not handle this": without it the shell receives ^F, which on some
    // hosts is a real control character and not a search.
    expect(kept).toBe(false);
  });

  it("takes Command+F on a Mac, and leaves Ctrl+F to the shell there", () => {
    useAppStore().info = {
      name: "7Shell",
      version: "0.1.0",
      os: "macos",
      arch: "aarch64",
      debugBuild: false,
      protocolCount: 1,
      descriptorSchemaVersion: 1,
    };

    expect(keyHandler?.(key({ key: "f", metaKey: true }))).toBe(false);
    expect(keyHandler?.(key({ key: "f", ctrlKey: true })), "Ctrl+F stays the shell's on a Mac").toBe(true);
  });

  it("leaves a combination the binding does not name alone", () => {
    // Ctrl+F is the find key; Ctrl+Shift+F is not, and a combination the shell may want is not ours
    // to take.
    expect(keyHandler?.(key({ key: "f", ctrlKey: true, shiftKey: true }))).toBe(true);
  });

  it("passes everything else through", () => {
    expect(keyHandler?.(key({ key: "c" }))).toBe(true);
    expect(keyHandler?.(key({ key: "d", ctrlKey: true }))).toBe(true);
  });
});
