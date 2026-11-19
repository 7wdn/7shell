import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import TabStrip from "@/components/workspace/TabStrip.vue";
import { i18n } from "@/i18n";
import { usePreferencesStore } from "@/stores/preferences";
import { useTerminalsStore } from "@/stores/terminals";

let host: HTMLDivElement;
let app: App | undefined;

const session = {
  id: "session-1",
  name: "web",
  protocol: "ssh" as const,
  host: "example.test",
  port: 22,
  user: "root",
  auth: { method: "password" as const, hasSecret: true },
  terminal: {
    terminalType: "xterm-256color",
    keepaliveSeconds: 30,
    encoding: "utf-8" as const,
    backspace: "del" as const,
    forwardAgent: false,
    record: false,
  },
  updatedAt: 1_700_000_000,
};

async function mount(mode: "workspace" | "titlebar"): Promise<void> {
  const terminals = useTerminalsStore();
  terminals.tabs = [
    {
      kind: "terminal",
      id: "tab-1",
      handle: "handle-1",
      session,
      status: "open",
      failure: null,
      panel: null,
    },
  ];
  terminals.openedOrder = ["tab-1"];
  terminals.activeTabId = "tab-1";

  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(TabStrip, { mode }) }).use(i18n);
  app.mount(host);
  await nextTick();
}

describe("the tab strip inside the title bar", () => {
  beforeEach(() => {
    setActivePinia(createPinia());
  });

  afterEach(() => {
    app?.unmount();
    host.remove();
  });

  it("is one row with the window's drag room after the tabs", async () => {
    usePreferencesStore().tabLayout = "wrap";
    await mount("titlebar");

    const strip = host.querySelector<HTMLElement>("[data-workspace-tabs]");
    expect(strip?.getAttribute("data-tab-placement")).toBe("titleBar");
    expect(strip?.classList.contains("flex-wrap"), "the row is not given back by wrapping").toBe(false);
    const tab = strip?.querySelector<HTMLElement>("[data-tab]");
    expect(tab).not.toBeNull();
    expect(tab?.classList.contains("rounded-md"), "the tab is a rounded box").toBe(true);
    expect(tab?.classList.contains("border"), "with an edge of its own").toBe(true);
    expect(strip?.querySelector("[data-tauri-drag-region]")).not.toBeNull();
  });

  it("keeps the workspace's own edges out of the title bar", async () => {
    await mount("titlebar");

    const strip = host.querySelector<HTMLElement>("[data-workspace-tabs]");
    expect(strip?.classList.contains("order-1"), "there are no panes to be ordered against").toBe(false);
    expect(strip?.classList.contains("border-b"), "the bar draws its own edge").toBe(false);
  });
});
