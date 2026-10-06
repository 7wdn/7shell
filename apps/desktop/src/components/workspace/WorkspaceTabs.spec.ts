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
