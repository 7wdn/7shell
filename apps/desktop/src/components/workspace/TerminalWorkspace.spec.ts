import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const lifecycle = vi.hoisted(() => ({ mounted: [] as string[], unmounted: [] as string[] }));
vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: () => true }));
vi.mock("@/lib/backend/tauri", () => ({
  sessionOpen: vi.fn(async (sessionId: string) => ({
    kind: "opened",
    handle: "h-new",
    sessionId,
    tunnels: [],
    tunnelFailures: [],
  })),
  terminalClose: vi.fn(async () => {}),
  tunnelsOpen: vi.fn(async () => []),
}));
vi.mock("@/components/workspace/TerminalWorkspacePane.vue", async () => {
  const { defineComponent, h, onBeforeUnmount, onMounted } = await import("vue");
  return {
    default: defineComponent({
      props: ["tab", "active", "split", "width", "height"],
      setup(props) {
        onMounted(() => lifecycle.mounted.push(props.tab.id));
        onBeforeUnmount(() => lifecycle.unmounted.push(props.tab.id));
        return () => h("textarea", { "data-screen": props.tab.id, class: "xterm-helper-textarea" });
      },
    }),
  };
});

import TerminalWorkspace from "@/components/workspace/TerminalWorkspace.vue";
import { i18n } from "@/i18n";
import { sessionOpen } from "@/lib/backend/tauri";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";
import type { Session } from "@/types/session";

const session: Session = {
  id: "local",
  name: "Local",
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
let app: App | undefined;
let host: HTMLElement;
let announceResize: () => void;

beforeEach(async () => {
  setActivePinia(createPinia());
  vi.clearAllMocks();
  lifecycle.mounted.length = 0;
  lifecycle.unmounted.length = 0;
  usePreferencesStore().sessionLayout = "terminal";
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        announceResize = callback;
      }
      observe() {}
      disconnect() {}
    },
  );
  const store = useTerminalsStore();
  store.tabs = ["a", "b"].map((id) => ({
    kind: "terminal",
    id,
    session,
    handle: `h-${id}`,
    status: "open",
    failure: null,
    panel: null,
  }));
  store.activeTabId = "a";
  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(TerminalWorkspace) }).use(i18n);
  app.mount(host);
  const stage = host.querySelector<HTMLElement>("[data-terminal-workspace]")!;
  Object.defineProperty(stage, "clientWidth", { value: 1000, configurable: true });
  Object.defineProperty(stage, "clientHeight", { value: 600, configurable: true });
  announceResize();
  await nextTick();
});

afterEach(() => {
  app?.unmount();
  host.remove();
  vi.unstubAllGlobals();
});

function pane(id: string): HTMLElement {
  return host.querySelector<HTMLElement>(`[data-terminal-pane][data-tab-id="${id}"]`)!;
}

describe("the terminal workspace", () => {
  it("defers pointer focus until a pane control receives its click", async () => {
    const store = useTerminalsStore();
    store.movePaneIntoSplit("b", "a", "right");
    store.activatePane("a");
    await nextTick();
    const button = pane("b").querySelector<HTMLButtonElement>("[data-pane-close]")!;
    button.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    button.focus();
    expect(store.activeTabId).toBe("a");
    button.click();
    await vi.waitFor(() => expect(store.terminalTabs).toHaveLength(1));
    expect(store.activeTabId).toBe("a");
  });
  it("keeps the same screen DOM and instance while moving, zooming and detaching a pane", async () => {
    const store = useTerminalsStore();
    const first = pane("a").querySelector<HTMLTextAreaElement>("textarea")!;
    const second = pane("b").querySelector<HTMLTextAreaElement>("textarea")!;
    first.value = "first scrollback";
    second.value = "second scrollback";
    store.movePaneIntoSplit("b", "a", "right");
    await nextTick();
    expect(pane("a").querySelector("textarea")).toBe(first);
    expect(pane("b").querySelector("textarea")).toBe(second);
    store.togglePaneZoom("b");
    await nextTick();
    expect(pane("a").style.display).toBe("none");
    expect(pane("b").style.width).toBe("1000px");
    store.togglePaneZoom("b");
    store.detachPane("b");
    await nextTick();
    expect(pane("b").querySelector("textarea")).toBe(second);
    expect(second.value).toBe("second scrollback");
    expect(first.value).toBe("first scrollback");
    expect(lifecycle.mounted).toEqual(["a", "b"]);
    expect(lifecycle.unmounted).toEqual([]);
  });

  it("focuses the pane containing an input and cycles panes with the registered shortcut", async () => {
    const store = useTerminalsStore();
    store.movePaneIntoSplit("b", "a", "right");
    await nextTick();
    const field = pane("a").querySelector<HTMLTextAreaElement>("textarea")!;
    field.focus();
    expect(store.activeTerminalHandle()).toBe("h-a");
    const key = new KeyboardEvent("keydown", {
      key: "]",
      code: "BracketRight",
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(key);
    expect(key.defaultPrevented).toBe(true);
    expect(store.activeTerminalHandle()).toBe("h-b");
  });

  it("closes only the pane whose close control was pressed", async () => {
    const store = useTerminalsStore();
    store.movePaneIntoSplit("b", "a", "right");
    await nextTick();
    pane("a").querySelector<HTMLButtonElement>("[data-pane-close]")!.click();
    await vi.waitFor(() => expect(store.terminalTabs).toHaveLength(1));
    expect(pane("b").querySelector("textarea")).not.toBeNull();
    expect(lifecycle.unmounted).toEqual(["a"]);
  });

  it("uses rebound split shortcuts and leaves ordinary terminal keys untouched", async () => {
    const preferences = usePreferencesStore();
    preferences.keybindings = { "terminal.splitRight": "Mod+Shift+K" };
    const field = pane("a").querySelector<HTMLTextAreaElement>("textarea")!;
    const ordinary = new KeyboardEvent("keydown", { key: "e", ctrlKey: true, bubbles: true, cancelable: true });
    field.dispatchEvent(ordinary);
    expect(ordinary.defaultPrevented).toBe(false);
    const splitKey = new KeyboardEvent("keydown", {
      key: "K",
      code: "KeyK",
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    field.dispatchEvent(splitKey);
    expect(splitKey.defaultPrevented).toBe(true);
    await vi.waitFor(() => expect(useTerminalsStore().paneCount("a")).toBe(2));
    expect(sessionOpen).toHaveBeenCalledOnce();
  });
});
