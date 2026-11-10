/**
 * The container pane as it is met: the connection it opens, the rail that switches views, and the
 * fact that switching does not destroy what was in the other view.
 *
 * The store's own lifecycle has a suite; what is checked here is only what a wrongly wired pane
 * would do — never connect, connect twice, or unmount a list whose filter and scroll position
 * somebody expects to come back to.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  containerConnect: vi.fn(async () => "conn-1"),
  containerDisconnect: vi.fn(async () => {}),
  containerList: vi.fn(async () => [
    {
      id: "0123456789abcdef0123456789abcdef",
      name: "web",
      image: "nginx:latest",
      state: "running",
      status: "Up",
      ports: "",
      createdAt: "now",
    },
  ]),
  containerImages: vi.fn(async () => []),
  containerRuntimeInfo: vi.fn(async () => ({
    runtime: "docker",
    cpus: 2,
    memory: 1024,
    containersTotal: 1,
    containersByState: [{ state: "running", count: 1 }],
    imagesTotal: 0,
    imagesTagged: 0,
    imagesDangling: 0,
  })),
  containerNamespaces: vi.fn(async () => []),
  containerSetNamespace: vi.fn(async () => {}),
  containerAction: vi.fn(async () => {}),
  containerRename: vi.fn(async () => {}),
  containerCreate: vi.fn(async () => {}),
  containerInspect: vi.fn(),
  containerLogsStart: vi.fn(async () => "log-1"),
  containerStreamStop: vi.fn(async () => {}),
  containerExecOpen: vi.fn(async () => "pty-1"),
  containerImageInspect: vi.fn(),
  containerImageHistory: vi.fn(async () => []),
  containerImagePull: vi.fn(async () => "pull-1"),
  containerImagePush: vi.fn(async () => "push-1"),
  containerImageRemove: vi.fn(async () => {}),
  containerImagePrune: vi.fn(async () => {}),
  containerImageTag: vi.fn(async () => {}),
  containerImageCommit: vi.fn(),
  containerRegistryLogin: vi.fn(async () => {}),
  onContainerStreamLine: vi.fn(async () => () => {}),
  onContainerStreamEnd: vi.fn(async () => () => {}),
}));

import ContainerPane from "@/components/container/ContainerPane.vue";
import { i18n } from "@/i18n";
import { containerConnect } from "@/lib/backend/tauri";
import type { ContainerTab } from "@/stores/terminals";

const connectMock = vi.mocked(containerConnect);

let app: App | undefined;
let host: HTMLDivElement | undefined;

function tab(runtime: "docker" | "nerdctl"): ContainerTab {
  return {
    kind: "container",
    id: "tab-1",
    session: {
      id: "session-1",
      name: "lab engine",
      protocol: "container",
      host: "",
      port: 0,
      user: "",
      auth: { method: "password", hasSecret: false },
      terminal: { terminalType: "xterm-256color", keepaliveSeconds: 0, forwardAgent: false, record: false },
      container: { runtime, transport: "local" },
      updatedAt: 0,
    },
    connectionId: null,
  };
}

async function mount(engine: ContainerTab): Promise<void> {
  app = createApp({ render: () => h(ContainerPane, { tab: engine, active: true }) });
  app.use(i18n);
  const pinia = createPinia();
  setActivePinia(pinia);
  app.use(pinia);
  host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
  await vi.waitFor(() => {
    expect(connectMock).toHaveBeenCalled();
  });
  // The connection resolves, then the three reads follow it; let all of that settle.
  await vi.waitFor(() => {
    expect(host?.textContent).toContain("web");
  });
}

function railButton(label: string): HTMLButtonElement {
  const nav = document.querySelector('[aria-label="Container workspace"]');
  if (!nav) {
    throw new Error("the rail was not drawn");
  }
  const button = [...nav.querySelectorAll("button")].find((entry) => entry.textContent?.includes(label));
  if (!button) {
    throw new Error(`the rail has no “${label}” view`);
  }
  return button as HTMLButtonElement;
}

afterEach(() => {
  app?.unmount();
  host?.remove();
  app = undefined;
  host = undefined;
  vi.clearAllMocks();
});

beforeEach(() => {
  connectMock.mockResolvedValue("conn-1");
});

describe("container pane", () => {
  it("opens the engine once when it is first drawn and writes the connection onto the tab", async () => {
    const engine = tab("docker");
    await mount(engine);

    expect(connectMock).toHaveBeenCalledTimes(1);
    expect(connectMock).toHaveBeenCalledWith("session-1");
    expect(engine.connectionId).toBe("conn-1");
  });

  it("keeps every view mounted while only one is shown", async () => {
    await mount(tab("docker"));

    const containersTable = document.querySelector('table[aria-label="Containers"]');
    expect(containersTable).not.toBeNull();

    railButton("Images").click();
    await nextTick();

    // Hidden rather than destroyed: the container list is still in the document, which is what lets
    // a filter, a scroll position and a pull in progress survive the switch.
    expect(document.querySelector('table[aria-label="Containers"]')).not.toBeNull();
    expect(document.querySelector('table[aria-label="Images"]')).not.toBeNull();
  });

  it("offers the namespace picker only where the runtime has namespaces", async () => {
    await mount(tab("docker"));
    expect(document.querySelector('[aria-label="Namespace"]')).toBeNull();
  });

  it("draws the namespace picker for containerd", async () => {
    await mount(tab("nerdctl"));
    expect(document.querySelector('[aria-label="Namespace"]')).not.toBeNull();
  });
});
