/**
 * The two halves of the panel under a terminal, as a person reads them off the strip.
 *
 * The store's suite already says what opening and closing the panel does to the browser. What can
 * only be seen here is which half is drawn as the one on screen: the two buttons under the terminal
 * are how somebody answers "where am I", and switching to the commands leaves the browser open
 * underneath — the backend still holds its channel — so a panel record that is merely *there* must
 * not keep the files button lit while the commands are the half being shown.
 *
 * It is mounted over a tab the store is simply told about, with no browser handle: opening one is a
 * conversation with a server, which is the live runs' business rather than this file's.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  filesClose: vi.fn(async () => null),
  filesOpen: vi.fn(),
  snippetsOpen: vi.fn(async () => []),
  commandHistory: vi.fn(async () => []),
}));

import WorkspaceTabs from "@/components/workspace/WorkspaceTabs.vue";
import { i18n } from "@/i18n";
import { filesOpen } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import { type TerminalPanel, useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

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

const other: Session = { ...session, id: "session-2", name: "db", host: "db.example.test" };

const openPanel: TerminalPanel = { handle: null, status: "opening", failure: null };

let host: HTMLDivElement;
let app: App | undefined;

/** Mounts the workspace over the tabs a case needs, with the first of them on screen. */
async function renderTabs(
  tabs: { kind: "terminal"; id: string; session: Session; panel?: TerminalPanel | null }[],
): Promise<void> {
  const terminals = useTerminalsStore();
  terminals.tabs = tabs.map((tab) => ({
    kind: "terminal" as const,
    id: tab.id,
    handle: null,
    session: tab.session,
    status: "open" as const,
    failure: null,
    panel: tab.panel ?? null,
  }));
  terminals.activeTabId = tabs[0]?.id ?? null;

  app = createApp({ render: () => h(WorkspaceTabs) }).use(i18n);
  app.mount(host);
  await nextTick();
}

/** Mounts the workspace over one open terminal, with or without a file panel under it. */
async function render(panel: TerminalPanel | null): Promise<void> {
  await renderTabs([{ kind: "terminal", id: "tab-1", session, panel }]);
}

function stripButton(label: string): HTMLElement {
  const found = [...host.querySelectorAll<HTMLElement>("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  if (!found) {
    throw new Error(`the strip has no button saying ${label}`);
  }
  return found;
}

/** The first tab in the strip, which is the drawn box rather than the button inside it. */
function firstTab(): HTMLElement {
  const found = host.querySelector<HTMLElement>("[data-workspace-tabs] > div");
  if (!found) {
    throw new Error("the strip has no tab in it");
  }
  return found;
}

/** The strip the tabs are drawn in, which is what the bar's own shape is read off. */
function strip(): HTMLElement {
  const found = host.querySelector<HTMLElement>("[data-workspace-tabs]");
  if (!found) {
    throw new Error("the workspace has no tab strip");
  }
  return found;
}

/** What a button says about being the half on screen, which is what its own lit state means. */
function pressed(button: HTMLElement): string | null {
  return button.getAttribute("aria-pressed");
}

async function click(button: HTMLElement): Promise<void> {
  button.click();
  await nextTick();
}

/** Whether the file half is the one being drawn — the half with no browser says it is opening. */
function filesHalfDrawn(): boolean {
  return (host.textContent ?? "").includes("Opening the file panel");
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.append(host);
  vi.mocked(filesOpen).mockClear();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the half of the panel that is on screen", () => {
  it("stops the files button saying so once the commands are the half being shown", async () => {
    await render(openPanel);

    // An open panel starts on the files, which is what the lit button has to say before anything
    // is switched.
    expect(pressed(stripButton("Files"))).toBe("true");
    expect(filesHalfDrawn()).toBe(true);

    await click(stripButton("Commands"));

    expect(pressed(stripButton("Commands"))).toBe("true");
    expect(pressed(stripButton("Files"))).toBe("false");
    expect(filesHalfDrawn()).toBe(false);

    // And back: the browser was never let go of, so the files are drawn again without a second
    // one being opened for the same session.
    await click(stripButton("Files"));

    expect(pressed(stripButton("Files"))).toBe("true");
    expect(pressed(stripButton("Commands"))).toBe("false");
    expect(filesHalfDrawn()).toBe(true);
    expect(filesOpen).not.toHaveBeenCalled();
  });

  it("shows the commands with no browser open, and opens none behind them", async () => {
    await render(null);
    expect(pressed(stripButton("Files"))).toBe("false");

    await click(stripButton("Commands"));

    expect(pressed(stripButton("Commands"))).toBe("true");
    expect(pressed(stripButton("Files"))).toBe("false");
    expect(filesOpen).not.toHaveBeenCalled();
  });

  it("closes the panel when the files are the half showing and their own button is pressed", async () => {
    await render(openPanel);

    await click(stripButton("Files"));

    // Both buttons are off, because the panel itself is gone rather than switched.
    expect(pressed(stripButton("Files"))).toBe("false");
    expect(pressed(stripButton("Commands"))).toBe("false");
    expect(filesHalfDrawn()).toBe(false);
  });
});

/**
 * The strips are chrome rather than text.
 *
 * A tab name is something to press, and the line under the terminal says where the session is and
 * holds the way into the file panel. Left to itself the browser treats both as a document: a drag
 * along the strip smears a selection over the names, and the pointer turns into an I-beam over them.
 * `docs/ui-conventions.md` states the rule for the whole interface; this is the case that would
 * notice it going missing from the elements this file draws, which the file-level guard cannot see —
 * both strips live in one component, and either could lose the class alone.
 */
/**
 * Where the bar is drawn, what it groups by, what order it draws and how wide.
 *
 * The four choices about the bar itself, all of which change what somebody sees without changing any
 * tab: which edge it is against (and therefore whether it is a row or a column), whether the tabs are
 * cut into runs by session, the order they are drawn in, and how wide one may be. What the *scrolling*
 * choice does is the case above; what the store does with a move or a close is the store's own suite.
 */
describe("the bar's own choices", () => {
  it("draws the bar as a column down the edge it was given", async () => {
    usePreferencesStore().setTabPlacement("left");
    await render(openPanel);

    const section = host.querySelector("section");
    expect(section?.classList.contains("flex-row"), "a column beside the panes").toBe(true);
    expect(strip().classList.contains("flex-col"), "with the tabs stacked in it").toBe(true);
    expect(strip().classList.contains("border-r"), "against the left edge").toBe(true);
  });

  it("puts the bar after the panes at the far edges and before them across the top", async () => {
    await render(openPanel);
    expect(strip().classList.contains("order-1"), "the bar is the way in, so it comes first").toBe(true);

    usePreferencesStore().setTabPlacement("bottom");
    await render(openPanel);
    expect(strip().classList.contains("order-2"), "at the bottom it is drawn after the pane").toBe(true);
    expect(strip().classList.contains("border-t"), "and the line between them is its own top edge").toBe(true);
  });

  it("names each session once when the tabs are grouped by one", async () => {
    usePreferencesStore().setTabGroupMode("session");
    await renderTabs([
      { kind: "terminal", id: "tab-1", session },
      { kind: "terminal", id: "tab-2", session },
      { kind: "terminal", id: "tab-3", session: other },
    ]);

    // One heading per session, named after it, in the order the sessions first appear — and the two
    // tabs of the same session share one heading rather than getting one each.
    const headings = [...host.querySelectorAll<HTMLElement>("[data-tab-group]")].map((entry) =>
      entry.textContent?.trim(),
    );
    expect(headings).toEqual(["web", "db"]);
  });

  it("draws no heading at all when nothing is grouped", async () => {
    await renderTabs([
      { kind: "terminal", id: "tab-1", session },
      { kind: "terminal", id: "tab-2", session: other },
    ]);

    expect(host.querySelectorAll("[data-tab-group]")).toHaveLength(0);
  });

  it("orders the tabs by name when that is the choice", async () => {
    usePreferencesStore().setTabSortMode("title-asc");
    await renderTabs([
      { kind: "terminal", id: "tab-1", session: other },
      { kind: "terminal", id: "tab-2", session },
    ]);

    const drawn = [...host.querySelectorAll<HTMLElement>("[data-tab]")].map((tab) => tab.dataset.tab);
    expect(drawn, "db before web, whatever order they were opened in").toEqual(["tab-1", "tab-2"]);
  });

  it("draws a tab no wider than the step it was given", async () => {
    usePreferencesStore().setTabMaxWidth("narrow");
    await render(openPanel);
    expect(firstTab().style.maxWidth, "the narrowest step").toBe("160px");

    usePreferencesStore().setTabMaxWidth("unlimited");
    await renderTabs([{ kind: "terminal", id: "tab-1", session, panel: openPanel }]);
    expect(firstTab().style.maxWidth, "and nothing at all where the name sets the width").toBe("");
  });

  it("offers a tab for dragging only while the order on screen is the manual one", async () => {
    await render(openPanel);
    expect(firstTab().getAttribute("draggable"), "the order a drag writes to").toBe("true");

    usePreferencesStore().setTabSortMode("created-asc");
    await renderTabs([{ kind: "terminal", id: "tab-1", session, panel: openPanel }]);
    expect(firstTab().getAttribute("draggable"), "a move into a list nobody is looking at").toBe("false");
  });
});

/**
 * The two interface layouts, as the strip draws them.
 *
 * The classic one is what this build has always drawn: one run of tabs joined to the top edge, told
 * apart by the line along the bottom of the one on screen. The separated one floats every tab as a box
 * with air around it and marks the lit one by a brighter edge, because a gap has no bottom line to
 * draw. What each tab *says* does not change, so this is the whole of the difference the component is
 * responsible for; the panels' and the headers' halves are the stylesheet's.
 */
describe("the two interface layouts", () => {
  it("draws one run of tabs when the chrome is classic, which is the default", async () => {
    usePreferencesStore().setAppLayout("classic");
    await render(openPanel);

    expect(firstTab().className, "no box, no rounding").not.toContain("rounded");
    expect(firstTab().classList.contains("border-b-foreground/60"), "the lit tab is a line along its bottom").toBe(
      true,
    );
    expect(strip().classList.contains("gap-0"), "the tabs are edge to edge").toBe(true);
  });

  it("draws every tab as a box of its own when the chrome is separated", async () => {
    usePreferencesStore().setAppLayout("separated");
    await render(openPanel);

    expect(firstTab().classList.contains("rounded-md"), "a box with rounded corners").toBe(true);
    expect(strip().classList.contains("gap-1.5"), "and air around it").toBe(true);
  });
});

/**
 * What the bar does when the tabs no longer fit in a row.
 *
 * It is a trade rather than a look: one row that scrolls keeps the bar's height and puts the tabs past
 * the edge, while wrapping put every tab on screen and takes the height out of the panel below. What
 * this file can see is which of the two the bar was drawn as; that the panel below gives up the room
 * is a real window's measurement.
 */
describe("the tab bar when the tabs do not fit", () => {
  it("wraps them onto as many rows as they need when it is told to", async () => {
    usePreferencesStore().setTabLayout("wrap");
    await render(openPanel);

    expect(strip().classList.contains("flex-wrap"), "the tabs wrap").toBe(true);
    expect(strip().classList.contains("content-start"), "and the rows start at the top").toBe(true);
    expect(strip().classList.contains("overflow-visible"), "with nothing hidden past the edge").toBe(true);
  });

  it("keeps them in one row that scrolls by default", async () => {
    await render(openPanel);

    expect(strip().className, "no wrapping").not.toContain("flex-wrap");
    expect(strip().classList.contains("overflow-x-auto"), "one row, scrolled sideways").toBe(true);
  });
});

describe("the workspace's own strips", () => {
  it("keeps the tabs and the terminal's strip from being read as text", async () => {
    await render(openPanel);

    const strips = [
      { what: "the tab strip", selector: "[data-workspace-tabs]" },
      { what: "the terminal's strip", selector: "[data-terminal-strip]" },
    ] as const;

    for (const { what, selector } of strips) {
      const drawn = host.querySelector(selector);
      if (!drawn) {
        throw new Error(`${what} is not drawn`);
      }
      expect(drawn.classList.contains("select-none"), `${what} is not selectable`).toBe(true);
      expect(drawn.classList.contains("cursor-default"), `${what} shows no I-beam`).toBe(true);
    }
  });
});
