/**
 * The detail drawer's YAML tab, as it is met with a pointer.
 *
 * The shape of the payload and the list refresh are the store's business and are tested beside it;
 * what is only visible here is the drawer's manners — that Edit opens an editor holding the object's
 * own document, that Save sends what was typed as a JSON body to the object's own path, that a
 * rejection keeps the draft on screen, and that a document that does not parse never reaches the
 * cluster at all.
 */
import { type Pinia, createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  k8sConnect: vi.fn(),
  k8sDisconnect: vi.fn(),
  k8sRequest: vi.fn(),
  k8sWatchStart: vi.fn(),
  k8sStreamStop: vi.fn(),
  k8sLogsStart: vi.fn(),
  k8sExecOpen: vi.fn(),
  onK8sWatch: vi.fn(),
  onK8sStreamLine: vi.fn(),
  onK8sStreamEnd: vi.fn(),
  monitorSample: vi.fn(),
  monitorProcesses: vi.fn(),
  monitorSockets: vi.fn(),
  monitorProcessDetail: vi.fn(),
  sessionOpen: vi.fn(),
  terminalClose: vi.fn(),
  terminalWrite: vi.fn(),
  terminalResize: vi.fn(),
  knownHostTrust: vi.fn(),
  knownHostForget: vi.fn(),
  tunnelsOpen: vi.fn(async () => []),
  tunnelClose: vi.fn(),
  filesOpen: vi.fn(),
  filesList: vi.fn(),
  filesClose: vi.fn(),
  transfersOpen: vi.fn(),
}));

import K8sDetailDrawer from "@/components/k8s/K8sDetailDrawer.vue";
import { descriptorOf } from "@/components/k8s/resources";
import { i18n } from "@/i18n";
import { k8sConnect, k8sRequest, k8sStreamStop, k8sWatchStart, onK8sWatch } from "@/lib/backend/tauri";
import { useK8sStore } from "@/stores/k8s";
import type { K8sRequest, K8sResponse, K8sWatchEvent } from "@/types/k8s";
import type { Session } from "@/types/session";

const connectMock = vi.mocked(k8sConnect);
const requestMock = vi.mocked(k8sRequest);
const watchStartMock = vi.mocked(k8sWatchStart);
const streamStopMock = vi.mocked(k8sStreamStop);
const onWatchMock = vi.mocked(onK8sWatch);

const session: Session = {
  id: "session-k8s",
  name: "lab cluster",
  protocol: "kubernetes",
  host: "",
  port: 0,
  user: "",
  auth: { method: "password", hasSecret: false },
  terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
  kubernetes: { source: "defaultPath", context: "lab" },
  updatedAt: 1_700_000_000,
};

const configMap = {
  apiVersion: "v1",
  kind: "ConfigMap",
  metadata: { name: "settings", namespace: "default", uid: "uid-cm", resourceVersion: "7" },
  data: { mode: "old" },
};

function answer(status: number, body: unknown): K8sResponse {
  return { status, body: JSON.stringify(body) };
}

function list(body: unknown): K8sResponse {
  return answer(200, { items: [body], metadata: { resourceVersion: "7" } });
}

/** Lets the store's chained awaits run. */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
}

let host: HTMLDivElement;
let app: App | undefined;
let pinia: Pinia;

function t(key: string): string {
  return i18n.global.t(key);
}

/** A button by the words it wears, which is how it is found with a pointer too. */
function button(label: string): HTMLButtonElement {
  const found = [...host.querySelectorAll("button")].find((entry) => entry.textContent?.trim() === label);
  if (!found) {
    throw new Error(`no button says ${label}: ${host.textContent}`);
  }
  return found;
}

function editor(): HTMLTextAreaElement | null {
  return host.querySelector("textarea");
}

async function openEditor(): Promise<HTMLTextAreaElement> {
  button(t("k8s.detail.tabs.yaml")).click();
  await nextTick();
  button(t("k8s.yaml.edit")).click();
  await nextTick();
  const found = editor();
  if (!found) {
    throw new Error("the editor did not open");
  }
  return found;
}

async function type(value: string): Promise<void> {
  const found = editor();
  if (!found) {
    throw new Error("the editor closed while it was being typed in");
  }
  found.value = value;
  found.dispatchEvent(new Event("input"));
  await nextTick();
}

/** The drawer, holding an object the store is already following. */
async function mountDrawer(): Promise<void> {
  const store = useK8sStore();
  await store.connect(session);
  store.subscribe("conn-1", "configmaps", "default");
  await settle();

  const descriptor = descriptorOf("configmaps");
  if (!descriptor) {
    throw new Error("the configmaps descriptor is missing");
  }
  app = createApp({
    render: () =>
      h(K8sDetailDrawer, {
        open: true,
        object: configMap,
        descriptor,
        events: [],
        eventsFailure: null,
        eventsLoading: false,
      }),
  });
  app.use(pinia).use(i18n);
  app.mount(host);
  await nextTick();
}

beforeEach(() => {
  host = document.createElement("div");
  document.body.append(host);
  pinia = createPinia();
  setActivePinia(pinia);
  connectMock.mockResolvedValue("conn-1");
  watchStartMock.mockResolvedValue("stream-1");
  streamStopMock.mockResolvedValue(undefined);
  onWatchMock.mockImplementation(async (_listener: (event: K8sWatchEvent) => void) => () => undefined);
  requestMock.mockImplementation(async (_connId, request) =>
    request.path === "/api/v1/namespaces/default/configmaps?limit=500" ? list(configMap) : answer(200, { items: [] }),
  );
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  vi.clearAllMocks();
});

describe("saving an edited manifest", () => {
  it("sends the draft as a JSON body to the object's own path with the write acknowledged", async () => {
    await mountDrawer();
    const draft = await openEditor();
    expect(draft.value).toContain("mode: old");

    await type(`apiVersion: v1
kind: ConfigMap
metadata:
  name: settings
  namespace: default
data:
  mode: new
`);
    button(t("k8s.yaml.save")).click();

    await vi.waitFor(() => {
      expect(requestMock.mock.calls.some((call) => call[1].method === "put")).toBe(true);
    });
    const sent: K8sRequest | undefined = requestMock.mock.calls.find((call) => call[1].method === "put")?.[1];
    expect(sent?.path).toBe("/api/v1/namespaces/default/configmaps/settings");
    expect(sent?.contentType).toBe("application/json");
    expect(sent?.acknowledged).toBe(true);
    expect(JSON.parse(sent?.body ?? "{}")).toMatchObject({ kind: "ConfigMap", data: { mode: "new" } });

    await vi.waitFor(() => expect(editor()).toBeNull());
  });

  it("keeps the draft when the API server refuses it", async () => {
    requestMock.mockImplementation(async (_connId, request) => {
      if (request.method === "put") {
        return answer(422, { kind: "Status", reason: "Invalid", code: 422, message: "spec.mode is required" });
      }
      return request.path === "/api/v1/namespaces/default/configmaps?limit=500" ? list(configMap) : answer(200, {});
    });
    await mountDrawer();
    await openEditor();

    await type("apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: settings\n");
    button(t("k8s.yaml.save")).click();

    await vi.waitFor(() => expect(host.textContent).toContain("K8S-REQUEST-FAILED"));
    expect(editor()?.value).toContain("kind: ConfigMap");
  });

  it("never sends a document that does not parse, and keeps it on screen", async () => {
    await mountDrawer();
    await openEditor();

    await type("kind: [unclosed");
    button(t("k8s.yaml.save")).click();
    await nextTick();

    expect(requestMock.mock.calls.some(([, request]) => request.method === "put")).toBe(false);
    expect(host.textContent).toContain(t("k8s.yaml.invalid"));
    expect(editor()?.value).toBe("kind: [unclosed");
  });
});
