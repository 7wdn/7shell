/**
 * Loading an image archive as a person meets it: a button that exists only where the runtime can
 * take an archive, the native dialog it asks the host for, and the three outcomes that are easy to
 * get wrong — loaded, dismissed, and refused.
 *
 * The refusal is asserted to leave *nothing* on the transient surface: the store keeps the envelope
 * and the pane's banner is its one home, so a toast here would be the same failure said twice.
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
  containerImageInspect: vi.fn(),
  containerImageHistory: vi.fn(async () => []),
  containerImagePull: vi.fn(async () => "pull-1"),
  containerImagePush: vi.fn(async () => "push-1"),
  containerImageRemove: vi.fn(async () => {}),
  containerImagePrune: vi.fn(async () => {}),
  containerImageTag: vi.fn(async () => {}),
  containerImageLoad: vi.fn(async () => null),
  containerImageCommit: vi.fn(),
  containerRegistryLogin: vi.fn(async () => {}),
  onContainerStreamLine: vi.fn(async () => () => {}),
  onContainerStreamEnd: vi.fn(async () => () => {}),
}));

import ContainerImageView from "@/components/container/ContainerImageView.vue";
import { useToast } from "@/composables/useToast";
import { i18n } from "@/i18n";
import { containerImageLoad, containerImages } from "@/lib/backend/tauri";
import { type ContainerSession, loadedImages, useContainerStore } from "@/stores/container";
import { useTerminalsStore, type ContainerTab } from "@/stores/terminals";
import type { ContainerRuntime } from "@/types/container";

const loadMock = vi.mocked(containerImageLoad);
const imagesMock = vi.mocked(containerImages);
const toastState = useToast();

let app: App | undefined;
let host: HTMLDivElement | undefined;

function tab(runtime: ContainerRuntime): ContainerTab {
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

/** Opens the store's session for the tab and mounts the images view over it. */
async function mount(runtime: ContainerRuntime = "docker"): Promise<ContainerSession> {
  const engine = tab(runtime);
  const pinia = createPinia();
  setActivePinia(pinia);
  const store = useContainerStore();
  await store.connect(engine);
  const session = store.sessionFor(engine.id);
  if (!session) {
    throw new Error("the session was never opened");
  }
  useTerminalsStore();

  app = createApp({ render: () => h(ContainerImageView, { tab: engine, session }) });
  app.use(i18n);
  app.use(pinia);
  host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
  await nextTick();
  return session;
}

/** The load button, or nothing when this runtime is not offered one. */
function loadButton(): HTMLButtonElement | null {
  return host?.querySelector<HTMLButtonElement>('button[aria-label="Load an image from an archive"]') ?? null;
}

afterEach(() => {
  app?.unmount();
  host?.remove();
  app = undefined;
  host = undefined;
  vi.clearAllMocks();
});

beforeEach(() => {
  toastState.message.value = "";
  loadMock.mockResolvedValue(null);
  imagesMock.mockResolvedValue([]);
});

describe("loading an image archive", () => {
  it("is offered on the runtimes that read an archive and refused on the others", async () => {
    await mount("docker");
    expect(loadButton()?.textContent).toContain("Load image");
    app?.unmount();
    host?.remove();

    await mount("podman");
    expect(loadButton()).not.toBeNull();
    app?.unmount();
    host?.remove();

    await mount("nerdctl");
    expect(loadButton()).toBeNull();
    app?.unmount();
    host?.remove();

    await mount("wslc");
    expect(loadButton()).toBeNull();
  });

  it("asks the host for a file under the dialog title and reads the list again", async () => {
    await mount();
    loadMock.mockResolvedValue("Loaded image: busybox:1.36\n");
    const before = imagesMock.mock.calls.length;

    loadButton()?.click();
    await vi.waitFor(() => {
      expect(loadMock).toHaveBeenCalledWith("conn-1", "Load an image from an archive");
    });
    await vi.waitFor(() => {
      expect(imagesMock.mock.calls.length).toBe(before + 1);
    });
    await vi.waitFor(() => {
      expect(toastState.message.value).toBe("Loaded busybox:1.36.");
    });
  });

  it("says nothing when the dialog is dismissed", async () => {
    await mount();
    const before = imagesMock.mock.calls.length;

    loadButton()?.click();
    await vi.waitFor(() => {
      expect(loadMock).toHaveBeenCalled();
    });

    expect(toastState.message.value).toBe("");
    expect(imagesMock.mock.calls.length).toBe(before);
  });

  it("leaves a refusal to the pane's banner instead of sounding it twice", async () => {
    const session = await mount();
    loadMock.mockRejectedValue(new Error("the runtime said no"));

    loadButton()?.click();
    await vi.waitFor(() => {
      expect(session.error).not.toBeNull();
    });

    expect(toastState.message.value).toBe("");
    expect(loadButton()?.disabled).toBe(false);
  });

  it("turns the runtime's own sentence into the reference it loaded", () => {
    expect(loadedImages("Loaded image: busybox:1.36\n")).toEqual(["busybox:1.36"]);
    expect(loadedImages("Loaded image ID: sha256:abc123")).toEqual(["sha256:abc123"]);
    expect(loadedImages("Loaded image: a:1\nLoaded image: b:2\n")).toEqual(["a:1", "b:2"]);
    expect(loadedImages("   ")).toEqual([]);
  });
});
