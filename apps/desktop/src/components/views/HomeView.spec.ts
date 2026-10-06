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
}));

import HomeView from "@/components/views/HomeView.vue";
import { i18n } from "@/i18n";
import { sessionOpen, sessionsImport } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import { useRecentSessionsStore } from "@/stores/recentSessions";
import { useSessionEditorStore } from "@/stores/sessionEditor";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

const sessionOpenMock = vi.mocked(sessionOpen);
const sessionsImportMock = vi.mocked(sessionsImport);

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
  useSessionsStore().sessions = [alpha, beta, gamma];
  // The panel under a terminal is not this page's business: a click here opens the terminal, and
  // the store's own suite already says what it does about the file panel afterwards.
  usePreferencesStore().sessionLayout = "terminal";
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

    await vi.waitFor(() => expect(sessionsImportMock).toHaveBeenCalledWith("Import sessions"));
  });
});
