/**
 * The transfer panel, as a person meets it: what a row offers, and what pressing it does.
 *
 * The rules about files — what a part file is, when a retry continues — live in `sevenshell-sftp`
 * and are tested against a real server there. What can only be seen here is the wiring: that the
 * button on a row is the one thing that row needs next, that the two ways of being finished with a
 * transfer reach the calls they name, and that putting the panel away leaves the rows behind.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  transfersOpen: vi.fn(async () => []),
  transferStop: vi.fn(async () => {}),
  transferRemove: vi.fn(async () => {}),
  transferStopAll: vi.fn(async () => {}),
  transferClearSettled: vi.fn(async () => {}),
  transferOpen: vi.fn(async () => {}),
  transferReveal: vi.fn(async () => {}),
  transferPause: vi.fn(async () => {}),
  transferResume: vi.fn(async () => {}),
  transferRetry: vi.fn(async () => {}),
  downloadFolderGet: vi.fn(async () => ({ current: null, history: [] })),
  downloadFolderChoose: vi.fn(async () => undefined),
  downloadFolderSelect: vi.fn(async () => ({ current: "fsdownload", history: ["fsdownload"] })),
  downloadFolderAsk: vi.fn(async () => ({ current: null, history: ["fsdownload"] })),
  downloadFolderReveal: vi.fn(async () => {}),
}));

import TransferPanel from "@/components/shell/TransferPanel.vue";
import { useContextMenu } from "@/composables/useContextMenu";
import { i18n } from "@/i18n";
import { ShellError } from "@/lib/backend/errors";
import type { MenuAction } from "@/lib/menu/types";
import { downloadFolderAsk, downloadFolderGet, downloadFolderReveal, downloadFolderSelect } from "@/lib/backend/tauri";
import {
  transferClearSettled,
  transferOpen,
  transferPause,
  transferRemove,
  transferResume,
  transferRetry,
  transferStop,
  transferStopAll,
} from "@/lib/backend/tauri";
import { useFilesStore } from "@/stores/files";
import type { TransferInfo, TransferState } from "@/types/session";

const clearMock = vi.mocked(transferClearSettled);
const openMock = vi.mocked(transferOpen);
const pauseMock = vi.mocked(transferPause);
const removeMock = vi.mocked(transferRemove);
const resumeMock = vi.mocked(transferResume);
const retryMock = vi.mocked(transferRetry);
const stopMock = vi.mocked(transferStop);
const stopAllMock = vi.mocked(transferStopAll);
const folderGetMock = vi.mocked(downloadFolderGet);
const folderSelectMock = vi.mocked(downloadFolderSelect);
const folderAskMock = vi.mocked(downloadFolderAsk);
const folderRevealMock = vi.mocked(downloadFolderReveal);

let app: App | undefined;
let host: HTMLDivElement | undefined;

function transfer(id: string, state: TransferState, overrides: Partial<TransferInfo> = {}): TransferInfo {
  return {
    id,
    direction: "download",
    name: `${id}.txt`,
    remotePath: `/home/labuser/${id}.txt`,
    sessionId: "s1",
    size: 1024,
    transferred: 512,
    state,
    hasLocalPath: true,
    packed: false,
    ...overrides,
  };
}

/**
 * Draws the panel over rows the store is simply told about.
 *
 * The polling belongs to the store, and what this file is about is what a row offers — so the rows
 * are put in the store rather than waited for from a server.
 */
async function render(...rows: TransferInfo[]): Promise<void> {
  // A second call in one case is a redraw rather than a second application: what was drawn first is
  // taken down, or its listeners would go on answering for a panel nobody can see.
  app?.unmount();
  host?.remove();

  const pinia = createPinia();
  setActivePinia(pinia);

  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(TransferPanel) })
    .use(pinia)
    .use(i18n);

  const files = useFilesStore();
  files.transfers = [...rows];
  files.panelExpanded = true;
  app.mount(host);
  await nextTick();
}

/** What was drawn, which every helper here reads from. */
function drawn(): HTMLDivElement {
  if (!host) {
    throw new Error("nothing is drawn: render() comes first");
  }
  return host;
}

/**
 * The one button on a row, by the name it answers to.
 *
 * The buttons are icons whose words are tooltips and accessible names, so what a test can hold them
 * by is the name rather than what is drawn — which is the same thing a screen reader reads.
 */
function buttonFor(label: string): HTMLButtonElement {
  const button = [...drawn().querySelectorAll<HTMLButtonElement>("[data-transfer-row] button")].find(
    (candidate) => candidate.getAttribute("aria-label") === label,
  );
  if (!button) {
    throw new Error(`no button reading ${label}`);
  }
  return button;
}

/** The panel's own header button, by its accessible name. */
function headerButton(name: string): HTMLButtonElement {
  const button = [...drawn().querySelectorAll<HTMLButtonElement>("header button")].find(
    (candidate) => candidate.getAttribute("aria-label") === name || candidate.getAttribute("title") === name,
  );
  if (!button) {
    throw new Error(`no header button named ${name}`);
  }
  return button;
}

/** A button of the question the shell draws, which is teleported out of the panel. */
function confirmButton(label: string): HTMLButtonElement | undefined {
  return [...document.querySelectorAll<HTMLButtonElement>("[role='dialog'] button")].find(
    (candidate) => candidate.textContent?.trim() === label,
  );
}

function labels(): string[] {
  return [...drawn().querySelectorAll<HTMLButtonElement>("[data-transfer-row] button")].map(
    (button) => button.getAttribute("aria-label") ?? "",
  );
}

beforeEach(() => {
  clearMock.mockClear();
  openMock.mockClear();
  pauseMock.mockClear();
  removeMock.mockClear();
  resumeMock.mockClear();
  retryMock.mockClear();
  stopMock.mockClear();
  stopAllMock.mockClear();
  folderGetMock.mockClear();
  folderSelectMock.mockClear();
  folderAskMock.mockClear();
  folderRevealMock.mockClear();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host?.remove();
  host = undefined;
});

describe("a transfer's row", () => {
  it("offers what the row needs next, and nothing else", async () => {
    await render(
      transfer("t1", { kind: "running" }),
      transfer("t2", { kind: "paused" }),
      transfer("t3", { kind: "failed", detail: "the channel failed" }),
      transfer("t4", { kind: "done" }),
      transfer("t5", { kind: "cancelled" }),
    );

    expect(labels()).toEqual(["Hold", "Continue", "Try again", "Remove", "Remove"]);
  });

  it("draws that button as a glyph with its word kept for a tooltip and a reader", async () => {
    await render(transfer("t1", { kind: "running" }));

    const button = buttonFor("Hold");
    expect(button.textContent?.trim(), "the word is not on the row taking width from the name").toBe("");
    expect(button.getAttribute("title")).toBe("Hold");
    expect(button.querySelector("svg"), "and what is drawn is an icon").not.toBeNull();
  });

  it("holds a running transfer, and lets a paused one go", async () => {
    await render(transfer("t1", { kind: "running" }));
    buttonFor("Hold").click();
    await vi.waitFor(() => expect(pauseMock).toHaveBeenCalledWith("t1"));

    // What the row says after the backend has done it: the same transfer, waiting.
    useFilesStore().transfers = [transfer("t1", { kind: "paused" })];
    await nextTick();

    buttonFor("Continue").click();
    await vi.waitFor(() => expect(resumeMock).toHaveBeenCalledWith("t1"));
  });

  it("tries a stopped transfer again", async () => {
    await render(transfer("t3", { kind: "failed", detail: "the channel failed" }));

    // The reason it stopped is on the row rather than behind a hover: it is the thing somebody needs
    // to decide whether trying again is worth it.
    expect(drawn().textContent).toContain("the channel failed");

    buttonFor("Try again").click();
    await vi.waitFor(() => expect(retryMock).toHaveBeenCalledWith("t3"));
  });

  it("removes a finished row, and stops nothing that is already over", async () => {
    await render(transfer("t4", { kind: "done" }));

    buttonFor("Remove").click();

    await vi.waitFor(() => expect(removeMock).toHaveBeenCalledWith("t4"));
    expect(stopMock).not.toHaveBeenCalled();
  });
});

describe("the panel itself", () => {
  it("clears the finished ones through the call that leaves the moving ones alone", async () => {
    await render(transfer("t4", { kind: "done" }), transfer("t1", { kind: "running" }));

    headerButton("Clear the 1 transfers that have finished").click();

    await vi.waitFor(() => expect(clearMock).toHaveBeenCalled());
  });

  it("asks before stopping everything, and stops everything only when answered", async () => {
    await render(transfer("t1", { kind: "running" }), transfer("t2", { kind: "paused" }));

    headerButton("Stop all").click();
    // The question is drawn by the shell's dialog, so what this waits for is its button — and the
    // backend is not asked until that one is answered.
    await vi.waitFor(() => expect(confirmButton("Stop all")).toBeDefined());
    expect(stopAllMock).not.toHaveBeenCalled();

    confirmButton("Stop all")?.click();
    await vi.waitFor(() => expect(stopAllMock).toHaveBeenCalled());
  });

  it("hands over the entries about the file itself, by id and never by path", async () => {
    await render(transfer("t4", { kind: "done" }));

    const row = drawn().querySelector<HTMLElement>("[data-transfer-row]");
    row?.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
    await nextTick();

    // The panel hands the menu over; where it is drawn is the shell's business, so what is checked
    // here is what the panel contributed.
    const { entries } = useContextMenu();
    const open = entries.value.find((entry): entry is MenuAction => entry.kind === "action" && entry.id === "open");
    expect(open?.labelKey).toBe("files.transfer.open");

    open?.run();
    await vi.waitFor(() => expect(openMock).toHaveBeenCalledWith("t4"));
  });

  it("tucks itself away when a click lands somewhere else, and keeps the rows", async () => {
    await render(transfer("t1", { kind: "running" }));

    document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    await nextTick();

    const files = useFilesStore();
    expect(files.panelExpanded).toBe(false);
    expect(files.transfers).toHaveLength(1);

    // And the pill is what is left: one click from the list again.
    const pill = drawn().querySelector<HTMLButtonElement>("[data-transfer-pill]");
    expect(pill?.textContent).toContain("1 transfers");
    pill?.click();
    await nextTick();
    expect(files.panelExpanded).toBe(true);
  });

  /**
   * The panel can be asked for with nothing to show.
   *
   * It used to appear only when a transfer did, which left the folder downloads land in invisible —
   * and unsettable — until somebody had already downloaded something. So an empty panel is a state
   * worth drawing, and what it draws is the strip rather than an apology.
   */
  it("opens with nothing to list, and says where downloads land", async () => {
    await render();

    expect(drawn().querySelector("[data-transfer-empty]")?.textContent).toContain("No transfers yet");
    expect(drawn().querySelector("[data-transfer-download-folder]")).not.toBeNull();
    expect(drawn().querySelector("[data-transfer-row]")).toBeNull();
  });

  it("stays out of the way when it has nothing to show and nobody asked", async () => {
    await render();
    useFilesStore().panelExpanded = false;
    await nextTick();

    expect(drawn().querySelector("[data-transfer-panel]")).toBeNull();
    expect(
      drawn().querySelector("[data-transfer-pill]"),
      "and there is no pill for an empty list either: the title bar's button is the way back",
    ).toBeNull();
  });

  it("says where downloads land, and where it is when nothing is set", async () => {
    await render(transfer("t1", { kind: "running" }));
    const strip = drawn().querySelector("[data-download-folder]");

    expect(folderGetMock, "the panel reads it when it arrives").toHaveBeenCalled();
    expect(strip?.textContent).toContain("Ask every time");

    // A folder chosen once: the strip shows its name, and the button that opens it appears.
    useFilesStore().downloadFolder = { current: "fsdownload", history: ["fsdownload", "Desktop"] };
    await nextTick();

    expect(strip?.textContent).toContain("fsdownload");
    expect(drawn().querySelector("[data-download-folder-open]")).not.toBeNull();
  });

  it("hands over the list of remembered folders, and reaches what each row means", async () => {
    await render(transfer("t1", { kind: "running" }));
    useFilesStore().downloadFolder = { current: "fsdownload", history: ["fsdownload", "Desktop"] };
    await nextTick();

    drawn()
      .querySelector<HTMLButtonElement>("[data-download-folder]")
      ?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await nextTick();

    const { entries } = useContextMenu();
    const rows = entries.value.filter((entry): entry is MenuAction => entry.kind === "action");
    expect(rows.map((entry) => entry.id)).toEqual(["folder-0", "folder-1", "choose", "ask"]);

    rows.find((entry) => entry.id === "folder-1")?.run();
    await vi.waitFor(() => expect(folderSelectMock).toHaveBeenCalledWith(1));

    rows.find((entry) => entry.id === "ask")?.run();
    await vi.waitFor(() => expect(folderAskMock).toHaveBeenCalled());
  });

  it("opens the download folder through the backend, which is the side that knows where it is", async () => {
    await render(transfer("t1", { kind: "running" }));
    useFilesStore().downloadFolder = { current: "fsdownload", history: ["fsdownload"] };
    await nextTick();

    drawn().querySelector<HTMLButtonElement>("[data-download-folder-open]")?.click();

    await vi.waitFor(() => expect(folderRevealMock).toHaveBeenCalled());
  });

  it("puts what is moving first and what has stopped after it, with nothing between them", async () => {
    await render(
      transfer("t4", { kind: "done" }),
      transfer("t1", { kind: "running" }),
      transfer("t5", { kind: "cancelled" }),
      transfer("t3", { kind: "failed", detail: "the channel failed" }),
    );

    const order = [...drawn().querySelectorAll<HTMLElement>("[data-transfer-row]")].map((row) =>
      row.textContent?.includes("t1.txt") ? "moving" : row.textContent?.includes("t3.txt") ? "failed" : "stopped",
    );

    // The order is the whole of the sorting rule, and it is what keeps a batch of finished rows from
    // burying the one that is still moving now that there is no header folding them away.
    expect(order).toEqual(["moving", "failed", "stopped", "stopped"]);
  });

  it("names the session a row belongs to only when there is more than one to tell apart", async () => {
    await render(transfer("t1", { kind: "running" }), transfer("t2", { kind: "done" }));
    expect(drawn().querySelector("[data-transfer-session]")).toBeNull();

    // Two sessions on the list: now the label is the only thing saying whose bytes these are — and a
    // session with no tab and no library entry falls back to its id, which is the honest answer rather
    // than a blank, because a transfer outlives the tab that started it.
    await render(
      transfer("t1", { kind: "running" }, { sessionId: "s-alpha" }),
      transfer("t2", { kind: "done" }, { sessionId: "s-unknown" }),
    );
    expect([...drawn().querySelectorAll("[data-transfer-session]")].map((node) => node.textContent?.trim())).toEqual([
      "s-alpha",
      "s-unknown",
    ]);
  });

  it("keeps a row to two lines, and gives the third one to a failure", async () => {
    await render(transfer("t4", { kind: "done" }));

    // After a row finishes, everything on it is a thing to recognise rather than to read: a green bar,
    // a tick, and the word on hover. The line that used to spell the state out again was the height
    // this card spent on every finished download.
    const row = drawn().querySelector("[data-transfer-row]");
    expect(row?.querySelector("[data-transfer-detail]")).toBeNull();
    expect(row?.querySelector("[title='Finished']")).not.toBeNull();
    expect(row?.textContent, "the state is not written out a second time").not.toContain("Finished");

    // A failure is the one state somebody has to read, so it keeps a line of its own.
    await render(transfer("t3", { kind: "failed", detail: "the channel failed" }));
    expect(drawn().querySelector("[data-transfer-detail]")?.textContent).toContain("the channel failed");
  });

  /**
   * The panel is a list of rows that are pressed, not a document.
   *
   * Left to itself the browser treats every name, size and state on it as text — a drag that lands
   * on the panel smears a selection over it, and the pointer turns into an I-beam over the numbers.
   * A drag that lands there was meant for the listing the panel floats over, and what is worth taking
   * away is on the row's own menu. The classes are what is checked, as the monitor panel's own spec
   * does for the same claim: a test document lays nothing out and no pointer is in it, so the
   * mechanism is the thing to hold still — the real window's measurement is in the end-to-end run.
   */
  it("refuses to be read as a document, pointer included", async () => {
    await render(transfer("t1", { kind: "running" }));

    const card = drawn().querySelector("[data-transfer-panel]");
    if (!card) {
      throw new Error("the panel is not there");
    }
    expect(card.classList.contains("select-none"), "a drag across the panel selects nothing").toBe(true);
    expect(card.classList.contains("cursor-default"), "and the pointer stays an arrow over the rows").toBe(true);
  });

  /**
   * The one exception, and the reason it is one: a failure is the only thing on the panel that is
   * meant to leave it — it goes into a bug report or a chat, and what the transfer layer said is the
   * part somebody has to quote. Everything else has a menu entry that copies it.
   */
  it("keeps what a failure said selectable, under a row and over the list", async () => {
    await render(transfer("t3", { kind: "failed", detail: "the channel failed" }));

    const detail = drawn().querySelector("[data-transfer-detail]");
    if (!detail) {
      throw new Error(`no failure is drawn: ${drawn().textContent}`);
    }
    expect(detail.classList.contains("select-text")).toBe(true);
    expect(detail.textContent).toContain("the channel failed");

    // The banner is the other error on the panel: the one about the list itself rather than about a
    // transfer in it.
    useFilesStore().transferFailure = new ShellError({
      version: 1,
      code: "APP-INTERNAL",
      messageKey: "errors.app.internal",
      messageParams: {},
      origin: { subsystem: "app" },
      outcome: "rejected",
    });
    await nextTick();

    const banner = drawn().querySelector("[data-transfer-problem]");
    if (!banner) {
      throw new Error(`no banner is drawn: ${drawn().textContent}`);
    }
    expect(banner.classList.contains("select-text")).toBe(true);
  });
});
