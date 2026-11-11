/**
 * The container list as a person meets it: which rows the default filter leaves, which row can be
 * entered, and what a destructive answer actually sends.
 *
 * The reason this is a component test rather than only a store one is the wiring the store cannot
 * see: the default "running only", an exec button that is disabled rather than failing, and the
 * confirmation whose answer is what carries `acknowledged` to the backend.
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
      id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      name: "web",
      image: "nginx:latest",
      state: "running",
      status: "Up",
      ports: "8080:80/tcp",
      createdAt: "now",
    },
    {
      id: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      name: "old",
      image: "nginx:latest",
      state: "exited",
      status: "Exited (0)",
      ports: "",
      createdAt: "then",
    },
  ]),
  containerImages: vi.fn(async () => []),
  containerRuntimeInfo: vi.fn(async () => null).mockRejectedValue(new Error("no info")),
  containerNamespaces: vi.fn(async () => []),
  containerSetNamespace: vi.fn(async () => {}),
  containerAction: vi.fn(async () => {}),
  containerRename: vi.fn(async () => {}),
  containerCreate: vi.fn(async () => {}),
  containerInspect: vi.fn(),
  containerLogsStart: vi.fn(async () => "log-1"),
  containerStreamStop: vi.fn(async () => {}),
  containerExecOpen: vi.fn(async () => "pty-1"),
  filesOpenContainer: vi.fn(async () => ({
    kind: "opened",
    handle: "container-f1",
    sessionId: "session-1:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    listing: { path: "/", entries: [] },
    packed: false,
  })),
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

import ContainerListView from "@/components/container/ContainerListView.vue";
import { i18n } from "@/i18n";
import { containerAction, containerExecOpen, filesOpenContainer } from "@/lib/backend/tauri";
import { useContainerStore, type ContainerSession } from "@/stores/container";
import { useTerminalsStore, type ContainerTab } from "@/stores/terminals";

const actionMock = vi.mocked(containerAction);
const execMock = vi.mocked(containerExecOpen);
const filesMock = vi.mocked(filesOpenContainer);

let app: App | undefined;
let host: HTMLDivElement | undefined;

function tab(): ContainerTab {
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
      container: { runtime: "docker", transport: "local" },
      updatedAt: 0,
    },
    connectionId: null,
  };
}

/** Opens the store's session for the tab and mounts the list over it. */
async function mount(): Promise<ContainerSession> {
  const engine = tab();
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useContainerStore();
  await store.connect(engine);
  const session = store.sessionFor(engine.id);
  if (!session) {
    throw new Error("the session was never opened");
  }

  app = createApp({
    render: () => h(ContainerListView, { tab: engine, session, imageFilter: null, stateFilter: null }),
  });
  app.use(i18n);
  app.use(pinia);
  host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
  await nextTick();
  return session;
}

function rows(): HTMLTableRowElement[] {
  return [...(host?.querySelectorAll<HTMLTableRowElement>("tbody tr") ?? [])];
}

function button(label: string): HTMLButtonElement {
  const element = host?.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  if (!element) {
    throw new Error(`there is no button called “${label}”`);
  }
  return element;
}

afterEach(() => {
  app?.unmount();
  host?.remove();
  app = undefined;
  host = undefined;
  vi.clearAllMocks();
});

beforeEach(() => {
  actionMock.mockResolvedValue(undefined);
  execMock.mockResolvedValue("pty-1");
  filesMock.mockClear();
});

describe("container list", () => {
  it("leaves room between the first column and the pane edge", async () => {
    await mount();
    const table = host?.querySelector("table");
    expect(table?.parentElement?.className).toContain("px-3");
  });

  it("shows running containers by default and all of them once the filter is off", async () => {
    await mount();
    expect(rows()).toHaveLength(1);
    expect(rows()[0]?.textContent).toContain("web");

    const runningOnly = host?.querySelector<HTMLInputElement>('input[type="checkbox"]');
    if (!runningOnly) {
      throw new Error("the running-only filter is not on the toolbar");
    }
    runningOnly.click();
    await nextTick();

    expect(rows()).toHaveLength(2);
  });

  it("will not offer a shell in a container that is not running", async () => {
    await mount();
    const runningOnly = host?.querySelector<HTMLInputElement>('input[type="checkbox"]');
    runningOnly?.click();
    await nextTick();

    expect(button("Open a shell in old").disabled).toBe(true);
    expect(button("Open a shell in web").disabled).toBe(false);
  });

  /**
   * A stopped container has no file system to list, so the action says so by being unavailable —
   * the same rule the shell button follows, for the same reason: a click that can only fail is
   * worse than a button that is visibly not offered.
   */
  it("offers a container's files only while the container is running", async () => {
    await mount();
    const runningOnly = host?.querySelector<HTMLInputElement>('input[type="checkbox"]');
    runningOnly?.click();
    await nextTick();

    expect(button("Browse the files of web").disabled).toBe(false);
    expect(button("Browse the files of old").disabled).toBe(true);
  });

  it("opens a container's files through the engine the workspace already holds", async () => {
    await mount();

    button("Browse the files of web").click();

    await vi.waitFor(() => {
      expect(filesMock).toHaveBeenCalledWith("conn-1", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "/");
    });
    await vi.waitFor(() => {
      const terminals = useTerminalsStore();
      expect(terminals.tabs.some((entry) => entry.kind === "files" && entry.handle === "container-f1")).toBe(true);
    });
  });

  it("opens a shell as an ordinary terminal tab on the handle the backend made", async () => {
    await mount();

    button("Open a shell in web").click();
    await vi.waitFor(() => {
      expect(execMock).toHaveBeenCalledWith("conn-1", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "sh");
    });
    await vi.waitFor(() => {
      const terminals = useTerminalsStore();
      expect(terminals.tabs.some((entry) => entry.kind === "terminal" && entry.handle === "pty-1")).toBe(true);
    });
  });

  it("asks before stopping and sends the answer as the acknowledgement", async () => {
    await mount();

    button("Stop web").click();
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("Stop the container “web”?");
    });

    const confirm = [...document.querySelectorAll<HTMLButtonElement>("button")].find(
      (element) => element.textContent?.trim() === "Stop",
    );
    if (!confirm) {
      throw new Error("the confirmation has no Stop button");
    }
    confirm.click();

    await vi.waitFor(() => {
      expect(actionMock).toHaveBeenCalledWith("conn-1", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "stop", true);
    });
  });
});
