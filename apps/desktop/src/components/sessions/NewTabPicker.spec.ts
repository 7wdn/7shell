import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: () => true }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionOpen: vi.fn(async (sessionId: string) => ({
    kind: "opened",
    handle: "h-new",
    sessionId,
    tunnels: [],
    tunnelFailures: [],
  })),
}));

import NewTabPicker from "@/components/sessions/NewTabPicker.vue";
import { i18n } from "@/i18n";
import { sessionOpen } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import { useSessionsStore } from "@/stores/sessions";
import { useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

const session: Session = {
  id: "local-a",
  groupId: "default",
  name: "Local A",
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
const other: Session = { ...session, id: "local-b", name: "Local B" };
let app: App | undefined;
let host: HTMLElement;

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  usePreferencesStore().sessionLayout = "terminal";
  const library = useSessionsStore();
  library.groups = [{ id: "default", name: "Default", order: 0, pinned: false }];
  library.sessions = [session, other, { ...session, id: "files-only", name: "Files only", protocol: "ftp" }];
  const store = useTerminalsStore();
  store.tabs = [session, other].map((session, index) => ({
    kind: "terminal",
    id: index === 0 ? "a" : "b",
    session,
    handle: `h-${index}`,
    status: "open",
    panel: null,
    failure: null,
  }));
  store.activeTabId = "a";
  host = document.createElement("div");
  document.body.append(host);
});

afterEach(() => {
  app?.unmount();
  host.remove();
});

async function render(mode: "tab" | "split" = "split"): Promise<void> {
  const store = useTerminalsStore();
  if (mode === "split") store.openSplitPicker("a");
  else store.openNewTabPicker();
  app = createApp({
    render: () => ((mode === "split" ? store.splitPicker : store.newTabPickerOpen) ? h(NewTabPicker, { mode }) : null),
  }).use(i18n);
  app.mount(host);
  await nextTick();
}

function button(selector: string): HTMLButtonElement {
  return document.querySelector<HTMLButtonElement>(selector)!;
}

describe("the split terminal picker", () => {
  it("moves an existing terminal in the chosen direction without authenticating again", async () => {
    await render();
    button('[data-split-direction="down"]').click();
    button('[data-split-existing-row][data-pane-id="b"]').click();
    await nextTick();
    const store = useTerminalsStore();
    expect(store.workspaceTabs).toHaveLength(1);
    expect(store.terminalWorkspaces[0]?.layout).toMatchObject({ direction: "down" });
    expect(store.activeTerminalHandle()).toBe("h-1");
    expect(store.splitPicker).toBeNull();
    expect(sessionOpen).not.toHaveBeenCalled();
  });

  it("offers only terminal sessions for a new pane and bypasses the ordinary reopen question", async () => {
    await render();
    button('[data-split-source="new"]').click();
    await nextTick();
    expect(document.querySelector('[data-split-new-row][data-session-id="files-only"]')).toBeNull();
    button('[data-split-new-row][data-session-id="local-b"]').click();
    const store = useTerminalsStore();
    await vi.waitFor(() => expect(store.paneCount("a")).toBe(2));
    expect(sessionOpen).toHaveBeenCalledExactlyOnceWith(other.id);
    expect(store.reopenQuestion).toBeNull();
    expect(store.terminalTabs).toHaveLength(3);
    expect(store.workspaceTabs).toHaveLength(2);
  });

  it("filters existing instances and lets Enter move the selected terminal", async () => {
    await render();
    const field = document.querySelector<HTMLInputElement>("[data-split-picker] input")!;
    field.value = "LOCAL B";
    field.dispatchEvent(new Event("input", { bubbles: true }));
    await nextTick();
    expect(document.querySelectorAll("[data-split-existing-row]")).toHaveLength(1);
    field.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    expect(useTerminalsStore().workspaceTabs).toHaveLength(1);
    expect(sessionOpen).not.toHaveBeenCalled();
  });

  it("cancels without changing terminals or opening a connection", async () => {
    await render();
    button('[data-split-picker] button[aria-label="Close"]').click();
    expect(useTerminalsStore().workspaceTabs).toHaveLength(2);
    expect(useTerminalsStore().splitPicker).toBeNull();
    expect(sessionOpen).not.toHaveBeenCalled();
  });

  it("keeps the global new-tab picker on its existing reopen behavior", async () => {
    usePreferencesStore().sessionReopen = "ask";
    await render("tab");
    expect(document.querySelector("[data-split-picker]")).toBeNull();
    button('[data-new-tab-row][data-session-id="local-b"]').click();
    await vi.waitFor(() => expect(useTerminalsStore().reopenQuestion?.session.id).toBe(other.id));
    expect(useTerminalsStore().workspaceTabs).toHaveLength(2);
    expect(sessionOpen).not.toHaveBeenCalled();
  });
});
