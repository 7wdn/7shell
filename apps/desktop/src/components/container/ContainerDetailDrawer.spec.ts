/**
 * The container drawer: what the detail says, and what the log view does with the lines it is given.
 *
 * The log view has three behaviours a store test cannot see — following the tail, pausing without
 * discarding what arrived, and letting go of the stream when it closes — and each of them is
 * observable here by the only evidence that matters: what is on screen and what was sent back.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  containerConnect: vi.fn(async () => "conn-1"),
  containerDisconnect: vi.fn(async () => {}),
  containerList: vi.fn(async () => []),
  containerImages: vi.fn(async () => []),
  containerRuntimeInfo: vi.fn(async () => {
    throw new Error("nothing to say");
  }),
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
  onContainerStreamLine: vi.fn(),
  onContainerStreamEnd: vi.fn(async () => () => {}),
}));

import ContainerDetailDrawer from "@/components/container/ContainerDetailDrawer.vue";
import { i18n } from "@/i18n";
import { containerInspect, containerLogsStart, containerStreamStop, onContainerStreamLine } from "@/lib/backend/tauri";
import { useContainerStore } from "@/stores/container";
import type { ContainerTab } from "@/stores/terminals";
import type { ContainerDetail, ContainerStreamLine, ContainerSummary } from "@/types/container";

const inspectMock = vi.mocked(containerInspect);
const logsStartMock = vi.mocked(containerLogsStart);
const streamStopMock = vi.mocked(containerStreamStop);
const lineMock = vi.mocked(onContainerStreamLine);

let app: App | undefined;
let host: HTMLDivElement | undefined;
let lineListener: ((payload: ContainerStreamLine) => void) | null = null;

function target(): ContainerSummary {
  return {
    id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    name: "web",
    image: "nginx:latest",
    state: "running",
    status: "Up",
    ports: "8080:80/tcp",
    createdAt: "now",
  };
}

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

function detail(): ContainerDetail {
  return {
    id: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    name: "web",
    image: "nginx:latest",
    state: "running",
    status: "Up 2 hours",
    exitCode: 0,
    oomKilled: false,
    pid: 4242,
    restartPolicy: "always",
    entrypoint: "/docker-entrypoint.sh",
    command: "nginx -g 'daemon off;'",
    workDir: "/",
    user: "root",
    networkMode: "bridge",
    ip: "172.17.0.2",
    gateway: "172.17.0.1",
    ports: [{ hostPort: "8080", containerPort: "80", protocol: "tcp" }],
    mounts: [],
    env: ["PATH=/usr/bin"],
    raw: '{"Id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","Name":"/web"}',
  };
}

async function mount(mode: "detail" | "logs"): Promise<void> {
  const pinia = createPinia();
  setActivePinia(pinia);
  await useContainerStore().connect(tab());

  app = createApp({ render: () => h(ContainerDetailDrawer, { mode, tabId: "tab-1", target: target() }) });
  app.use(i18n);
  app.use(pinia);
  host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
  await nextTick();
}

function button(label: string): HTMLButtonElement {
  const element = host
    ? [...host.querySelectorAll<HTMLButtonElement>("button")].find((entry) => entry.textContent?.trim() === label)
    : undefined;
  if (!element) {
    throw new Error(`there is no button called “${label}”`);
  }
  return element;
}

beforeEach(() => {
  lineListener = null;
  lineMock.mockImplementation(async (listener) => {
    lineListener = listener;
    return () => {};
  });
});

afterEach(() => {
  app?.unmount();
  host?.remove();
  app = undefined;
  host = undefined;
  vi.clearAllMocks();
});

describe("container detail drawer", () => {
  it("draws the fields of a container it read", async () => {
    inspectMock.mockResolvedValue(detail());
    await mount("detail");

    await vi.waitFor(() => {
      expect(host?.textContent).toContain("172.17.0.2");
    });
    expect(host?.textContent).toContain("nginx -g 'daemon off;'");
    expect(host?.textContent).toContain("8080:80");
  });

  it("reads the logs it opens and keeps the lines that arrived while paused", async () => {
    logsStartMock.mockResolvedValue("log-1");
    await mount("logs");

    await vi.waitFor(() => {
      expect(logsStartMock).toHaveBeenCalledWith("conn-1", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", 500, false);
    });
    if (!lineListener) {
      throw new Error("the drawer never subscribed to the stream");
    }

    lineListener({ streamId: "log-1", line: "2026-11-10T10:00:00Z hello" });
    await nextTick();
    expect(host?.textContent).toContain("hello");

    button("Pause").click();
    await nextTick();
    lineListener({ streamId: "log-1", line: "while paused" });
    await nextTick();
    expect(host?.textContent).not.toContain("while paused");

    button("Resume").click();
    await nextTick();
    expect(host?.textContent).toContain("while paused");
  });

  it("stops the stream when it closes", async () => {
    logsStartMock.mockResolvedValue("log-1");
    await mount("logs");
    await vi.waitFor(() => {
      expect(logsStartMock).toHaveBeenCalled();
    });
    // Let the start answer land so the drawer holds the stream id.
    await vi.waitFor(() => {
      expect(host?.textContent).toContain("Nothing has been written yet.");
    });

    app?.unmount();
    app = undefined;
    await vi.waitFor(() => {
      expect(streamStopMock).toHaveBeenCalledWith("log-1");
    });
  });
});
