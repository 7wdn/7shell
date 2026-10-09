/**
 * The home page, as a person reads it off the screen.
 *
 * Two things can only be seen here: that the quick-start list is *ordering* rather than membership —
 * the sessions used last come first and the rest of the library fills the list out — and that every
 * shortcut starts something without the sidebar, which is the whole reason the page has shortcuts.
 * What each action then does is the stores' own business and is proved in their suites.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionOpen: vi.fn(),
  sessionsImport: vi.fn(async () => undefined),
  snippetsOpen: vi.fn(async () => []),
  commandHistory: vi.fn(async () => []),
}));

import HomeView from "@/components/views/HomeView.vue";
import { i18n } from "@/i18n";
import { commandHistory, sessionOpen, sessionsImport } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import { useRecentSessionsStore } from "@/stores/recentSessions";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

const sessionOpenMock = vi.mocked(sessionOpen);
const sessionsImportMock = vi.mocked(sessionsImport);
const commandHistoryMock = vi.mocked(commandHistory);

function sample(id: string, name: string): Session {
  return {
    id,
    name,
    protocol: "ssh",
    host: `${name}.test`,
    port: 22,
    user: "root",
    auth: { method: "password", hasSecret: false },
    terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
    updatedAt: 1_700_000_000,
  };
}

const alpha = sample("a", "alpha");
const beta = sample("b", "beta");
const gamma = sample("c", "gamma");

let host: HTMLDivElement;
let app: App | undefined;

function render(): void {
  app = createApp({ render: () => h(HomeView) }).use(i18n);
  app.mount(host);
}

/** The card whose heading says this, which is how the page is read rather than by class name. */
function card(heading: string): HTMLElement {
  const found = [...host.querySelectorAll<HTMLElement>("section")].find(
    (section) => section.querySelector("h2")?.textContent?.trim() === heading,
  );
  if (!found) {
    throw new Error(`the page has no card headed ${heading}`);
  }
  return found;
}

/** The sessions the quick-start card offers, in the order it draws them. */
function quickSessionIds(): string[] {
  return [...card("Quick start").querySelectorAll<HTMLElement>("[data-quick-session]")].map(
    (row) => row.dataset.quickSession ?? "",
  );
}

function buttonIn(card: HTMLElement, label: string): HTMLElement {
  const found = [...card.querySelectorAll<HTMLElement>("button")].find(
    (button) => button.textContent?.trim() === label,
  );
  if (!found) {
    throw new Error(`no button saying ${label}`);
  }
  return found;
}

/** The one thing a quick-start row's button does, reached by the session it is a row for. */
function quickRow(sessionId: string): HTMLElement {
  const found = card("Quick start").querySelector<HTMLElement>(`[data-quick-session="${sessionId}"] button`);
  if (!found) {
    throw new Error(`the quick-start list has no row for ${sessionId}`);
  }
  return found;
}

/** The number a tile of the row across the top draws, by the key that row was built with. */
function tileReading(key: string): string {
  const found = host.querySelector<HTMLElement>(`[data-tile="${key}"] [data-tile-value]`);
  if (!found) {
    throw new Error(`the page has no ${key} tile`);
  }
  return found.textContent?.trim() ?? "";
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.append(host);
  localStorage.clear();
  sessionOpenMock.mockReset().mockResolvedValue({
    kind: "opened",
    handle: "h-alpha",
    sessionId: alpha.id,
    tunnels: [],
    tunnelFailures: [],
  });
  sessionsImportMock.mockReset().mockResolvedValue(undefined);
  commandHistoryMock.mockReset().mockResolvedValue([]);
  useSessionsStore().sessions = [alpha, beta, gamma];
  // The panel under a terminal is not this page's business: a click here opens the terminal, and
  // the store's own suite already says what it does about the file panel afterwards.
  usePreferencesStore().sessionLayout = "terminal";
  // Most of these cases are about the working page — the list, the counters, the shortcuts — and a fresh
  // install opens on the introduction instead. Each of the two pages has its own case below, and both
  // say which one they are about rather than relying on which one happens to be the default.
  usePreferencesStore().setHomeLayout("workspace");
});

afterEach(() => {
  app?.unmount();
  host.remove();
  app = undefined;
});

describe("the home page", () => {
  it("offers the sessions used last first, and fills the rest out from the library", async () => {
    useRecentSessionsStore().record(gamma.id);
    render();

    expect(quickSessionIds()).toEqual(["c", "a", "b"]);
  });

  it("offers the library's own order when nothing has been used yet", async () => {
    render();

    expect(quickSessionIds()).toEqual(["a", "b", "c"]);
  });

  it("opens a terminal for the row that was clicked", async () => {
    render();
    quickRow(beta.id).click();

    await vi.waitFor(() => expect(sessionOpenMock).toHaveBeenCalledWith(beta.id));
  });

  it("says there are no sessions rather than drawing an empty card", () => {
    useSessionsStore().sessions = [];
    render();

    expect(quickSessionIds()).toEqual([]);
    expect(card("Quick start").textContent).toContain("No sessions yet");
  });

  it("starts a new session, the settings page and a batch run from its shortcuts", async () => {
    render();
    const actions = card("Common operations");

    buttonIn(actions, "New session").click();
    await nextTick();
    expect(useSessionEditorStore().open).toBe(true);

    buttonIn(actions, "Settings").click();
    await nextTick();
    expect(useTerminalsStore().tabs.some((tab) => tab.kind === "settings")).toBe(true);

    buttonIn(actions, "Batch execution").click();
    await nextTick();
    expect(useTerminalsStore().tabs.some((tab) => tab.kind === "batch")).toBe(true);
  });

  it("imports from the shortcut, which is the only way in when the sidebar is put away", async () => {
    usePreferencesStore().sidebarOpen = false;
    render();
    buttonIn(card("Common operations"), "Import sessions…").click();

    // The second argument is the passphrase, and there is none yet: a file that turns out to be
    // sealed stops on the frame's own question and comes back here with the answer.
    await vi.waitFor(() => expect(sessionsImportMock).toHaveBeenCalledWith("Import sessions", null));
  });

  it("lists the commands that were sent, newest first", async () => {
    commandHistoryMock.mockResolvedValue([
      { command: "systemctl reload nginx", at: 1_700_000_100 },
      { command: "uptime", at: 1_700_000_000 },
    ]);
    render();

    await vi.waitFor(() => expect(card("History").textContent).toContain("systemctl reload nginx"));
    expect(card("History").textContent).toContain("uptime");
  });

  it("says nothing has been sent rather than drawing an empty list", async () => {
    render();

    await vi.waitFor(() => expect(card("History").textContent).toContain("Nothing sent yet."));
  });

  it("counts only the sessions that have a shell running", () => {
    useTerminalsStore().tabs = [
      { kind: "terminal", id: "tab-1", session: alpha, handle: "h-alpha", status: "open", failure: null, panel: null },
      { kind: "terminal", id: "tab-2", session: beta, handle: "h-beta", status: "ended", failure: null, panel: null },
    ];
    render();

    // Two tabs and one answer: a shell that ended is a session somebody had, not one they have.
    expect(tileReading("open")).toBe("1");
    expect(tileReading("sessions")).toBe("3");
  });
});

/**
 * Which of the two pages is drawn.
 *
 * The page is the surface somebody meets a hundred times, and what it should hold changes as they
 * learn the application: while nothing has been connected, what this is and how a first connection
 * goes are what the page is for — and once there are hosts, the hosts are. Two pages rather than one
 * page with parts switched off, because the parts belong together: the counters and the names of the
 * hosts are one answer, not two.
 */
describe("the two home pages", () => {
  it("draws the working page: what the library holds, the hosts, the actions and what was run", () => {
    usePreferencesStore().setHomeLayout("workspace");
    render();

    expect(card("Quick start")).toBeTruthy();
    expect(card("Common operations")).toBeTruthy();
    expect(card("History"), "what was run is this page's too").toBeTruthy();
    expect(host.textContent, "the counters are this page's").toContain("Sessions saved");
    expect(() => card("Getting started"), "and the explanation belongs to the other one").toThrow();
  });

  it("draws the introduction instead, with nothing that names or counts a connection", () => {
    usePreferencesStore().setHomeLayout("intro");

    render();

    expect(card("Getting started")).toBeTruthy();
    expect(card("What this build can and cannot do")).toBeTruthy();
    expect(host.textContent, "what it does is named in its own words").toContain("File workbench");
    expect(() => card("Quick start"), "no host is named on this page").toThrow();
    expect(host.textContent, "and nothing here counts one either").not.toContain("Sessions saved");
  });
});
