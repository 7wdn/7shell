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

const openPanel: TerminalPanel = { handle: null, status: "opening", failure: null };

let host: HTMLDivElement;
let app: App | undefined;

/** Mounts the workspace over one open terminal, with or without a file panel under it. */
async function render(panel: TerminalPanel | null): Promise<void> {
  const terminals = useTerminalsStore();
  terminals.tabs = [{ kind: "terminal", id: "tab-1", handle: null, session, status: "open", failure: null, panel }];
  terminals.activeTabId = "tab-1";

  app = createApp({ render: () => h(WorkspaceTabs) }).use(i18n);
  app.mount(host);
  await nextTick();
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
 * The two interface layouts, as the strip draws them.
 *
 * The separated one is what this build has always drawn: every tab a block with air around it. The
 * classic one joins them into a single run divided by lines, and tells the lit one apart with a line
 * along its top — there is no gap left to tell it apart by. What each tab *says* does not change, so
 * this is the whole of the difference the component is responsible for; the headers' half of the
 * choice is a token in the stylesheet.
 */
describe("the two interface layouts", () => {
  /** The first tab in the strip, which is the drawn box rather than the button inside it. */
  function firstTab(): HTMLElement {
    const found = host.querySelector<HTMLElement>("[data-workspace-tabs] > div");
    if (!found) {
      throw new Error("the strip has no tab in it");
    }
    return found;
  }

  function strip(): HTMLElement {
    const found = host.querySelector<HTMLElement>("[data-workspace-tabs]");
    if (!found) {
      throw new Error("the workspace has no tab strip");
    }
    return found;
  }

  it("draws tabs as blocks of their own when the chrome is separated", async () => {
    usePreferencesStore().setAppLayout("separated");
    await render(openPanel);

    expect(firstTab().classList.contains("rounded-t-md"), "a block with a rounded top").toBe(true);
    expect(strip().classList.contains("gap-1"), "and air between the blocks").toBe(true);
  });

  it("draws one run of tabs when the chrome is classic", async () => {
    usePreferencesStore().setAppLayout("classic");
    await render(openPanel);

    expect(firstTab().className, "no block, no rounding").not.toContain("rounded");
    expect(firstTab().classList.contains("border-t-primary"), "the lit tab is a line along its top").toBe(true);
    expect(strip().classList.contains("gap-0"), "the tabs are edge to edge").toBe(true);
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
