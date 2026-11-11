/**
 * The cluster store: what a list-and-watch really does, and what it refuses to do.
 *
 * The assertions are about the parts that go wrong quietly — a second subscriber that opens a second
 * watch, an answer from a reload that lands after a newer one, a watch the API server expired being
 * reported as a failure, a metrics server that is simply not installed. Each of them looks fine on
 * screen for a while, which is why they are tested rather than watched.
 */
import { createPinia, setActivePinia } from "pinia";
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
  // The terminals store is built by the exec test, and its own initialisation reads these.
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

import {
  k8sConnect,
  k8sDisconnect,
  k8sExecOpen,
  k8sRequest,
  k8sStreamStop,
  k8sWatchStart,
  onK8sWatch,
} from "@/lib/backend/tauri";
import { nameOf, uidOf } from "@/components/k8s/objects";
import { clearCustomResourceDescriptors, descriptorOf } from "@/components/k8s/resources";
import { K8sHttpError, cacheKeyOf, scopedNamespace, useK8sStore } from "@/stores/k8s";
import { useTerminalsStore } from "@/stores/terminals";
import type { K8sResponse, K8sWatchEvent } from "@/types/k8s";
import type { Session } from "@/types/session";

const connectMock = vi.mocked(k8sConnect);
const disconnectMock = vi.mocked(k8sDisconnect);
const requestMock = vi.mocked(k8sRequest);
const watchStartMock = vi.mocked(k8sWatchStart);
const streamStopMock = vi.mocked(k8sStreamStop);
const execOpenMock = vi.mocked(k8sExecOpen);
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

const pod = (name: string, uid: string, resourceVersion: string, extra: Record<string, unknown> = {}) => ({
  apiVersion: "v1",
  kind: "Pod",
  metadata: { name, namespace: "default", uid, resourceVersion },
  status: { phase: "Running", containerStatuses: [{ ready: true, restartCount: 0 }] },
  spec: { nodeName: "node-a", containers: [{ name: "main", image: "nginx:1.27" }] },
  ...extra,
});

/** A definition whose kind the tree should be able to list after the connection reads it. */
const widgetCrd = {
  apiVersion: "apiextensions.k8s.io/v1",
  kind: "CustomResourceDefinition",
  metadata: { name: "widgets.example.com" },
  spec: {
    group: "example.com",
    names: { plural: "widgets", kind: "Widget" },
    scope: "Namespaced",
    versions: [
      {
        name: "v1",
        served: true,
        storage: true,
        additionalPrinterColumns: [{ name: "Size", jsonPath: ".spec.size" }],
      },
    ],
  },
};

function answer(status: number, body: unknown): K8sResponse {
  return { status, body: JSON.stringify(body) };
}

function listOf(items: readonly unknown[], resourceVersion: string): K8sResponse {
  return answer(200, { items, metadata: { resourceVersion } });
}

/** Lets the store's chained awaits run. Timers flush microtasks, which plain promises do not. */
async function settle(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 0));
  await new Promise((resolve) => setTimeout(resolve, 0));
}

/** The one watch callback the store registered, so a test can hand it an event. */
let watchListener: ((event: K8sWatchEvent) => void) | null = null;

beforeEach(() => {
  setActivePinia(createPinia());
  watchListener = null;
  connectMock.mockResolvedValue("conn-1");
  disconnectMock.mockResolvedValue(undefined);
  watchStartMock.mockResolvedValue("stream-1");
  streamStopMock.mockResolvedValue(undefined);
  onWatchMock.mockImplementation(async (listener: (event: K8sWatchEvent) => void) => {
    watchListener = listener;
    return () => undefined;
  });
  // The namespace list is read once per connection; a test that is not about it gets nothing.
  requestMock.mockImplementation(async (_connId, request) => {
    if (request.path.startsWith("/api/v1/namespaces")) {
      return listOf([], "1");
    }
    return listOf([], "1");
  });
});

afterEach(() => {
  vi.clearAllMocks();
  // The discovered kinds are module state shared by every store instance in this file.
  clearCustomResourceDescriptors("conn-1");
});

/** Connects the session and returns the store, which most tests need before anything else. */
async function connected() {
  const store = useK8sStore();
  await store.connect(session);
  await settle();
  return store;
}

describe("the list-and-watch cache", () => {
  it("lists once and watches from the version the list answered with", async () => {
    requestMock.mockImplementation(async (_connId, request) =>
      request.path === "/api/v1/pods?limit=500" ? listOf([pod("web-0", "uid-0", "11")], "11") : listOf([], "1"),
    );

    const store = await connected();
    store.subscribe("conn-1", "pods", "");
    await settle();

    const listCalls = requestMock.mock.calls.filter(([, request]) => request.path === "/api/v1/pods?limit=500");
    expect(listCalls).toHaveLength(1);
    expect(store.rowsOf("conn-1", "pods", "")).toHaveLength(1);
    expect(watchStartMock).toHaveBeenCalledTimes(1);
    expect(watchStartMock.mock.calls[0]?.[1]).toContain("resourceVersion=11");
    expect(watchStartMock.mock.calls[0]?.[1]).toContain("allowWatchBookmarks=true");
  });

  it("shares one watch between two subscribers and keeps it until the last one leaves", async () => {
    requestMock.mockImplementation(async (_connId, request) =>
      request.path === "/api/v1/pods?limit=500" ? listOf([pod("web-0", "uid-0", "11")], "11") : listOf([], "1"),
    );

    const store = await connected();
    store.subscribe("conn-1", "pods", "");
    store.subscribe("conn-1", "pods", "");
    await settle();

    expect(watchStartMock).toHaveBeenCalledTimes(1);
    store.unsubscribe("conn-1", "pods", "");
    expect(streamStopMock).not.toHaveBeenCalled();
    store.unsubscribe("conn-1", "pods", "");
    expect(streamStopMock).toHaveBeenCalledWith("stream-1");
    expect(store.rowsOf("conn-1", "pods", "")).toHaveLength(0);
  });

  it("upserts and deletes rows by uid, and follows the version a bookmark carries", async () => {
    requestMock.mockImplementation(async (_connId, request) =>
      request.path === "/api/v1/pods?limit=500" ? listOf([pod("web-0", "uid-0", "11")], "11") : listOf([], "1"),
    );

    const store = await connected();
    store.subscribe("conn-1", "pods", "");
    await settle();
    expect(watchListener).not.toBeNull();

    watchListener?.({
      streamId: "stream-1",
      kind: "ADDED",
      object: JSON.stringify(pod("web-1", "uid-1", "12")),
    });
    expect(store.rowsOf("conn-1", "pods", "").map((obj) => obj.metadata)).toEqual([
      { name: "web-0", namespace: "default", uid: "uid-0", resourceVersion: "11" },
      { name: "web-1", namespace: "default", uid: "uid-1", resourceVersion: "12" },
    ]);

    watchListener?.({
      streamId: "stream-1",
      kind: "BOOKMARK",
      object: JSON.stringify({ metadata: { resourceVersion: "13" } }),
    });
    watchListener?.({
      streamId: "stream-1",
      kind: "MODIFIED",
      object: JSON.stringify(pod("web-1", "uid-1", "14", { status: { phase: "Pending" } })),
    });
    expect(store.rowsOf("conn-1", "pods", "")[1]).toMatchObject({ status: { phase: "Pending" } });

    watchListener?.({ streamId: "stream-1", kind: "DELETED", object: JSON.stringify(pod("web-1", "uid-1", "15")) });
    expect(store.rowsOf("conn-1", "pods", "").map(uidOf)).toEqual(["uid-0"]);
  });

  it("stops a watch that finished opening after its last subscriber left", async () => {
    requestMock.mockImplementation(async (_connId, request) =>
      request.path === "/api/v1/pods?limit=500" ? listOf([], "11") : listOf([], "1"),
    );
    const watchGate: { resolve?: (streamId: string) => void } = {};
    watchStartMock.mockImplementation(
      () =>
        new Promise<string>((resolve) => {
          watchGate.resolve = resolve;
        }),
    );

    const store = await connected();
    store.subscribe("conn-1", "pods", "");
    await settle();

    // The view goes away while the watch is still being opened.
    store.unsubscribe("conn-1", "pods", "");
    expect(store.caches[cacheKeyOf("conn-1", "pods", "")]).toBeUndefined();

    watchGate.resolve?.("stream-late");
    await settle();

    expect(streamStopMock).toHaveBeenCalledWith("stream-late");
  });
});

describe("a watch the API server ends", () => {
  it("relists from scratch when the version expired, instead of showing an error", async () => {
    let lists = 0;
    requestMock.mockImplementation(async (_connId, request) => {
      if (request.path === "/api/v1/pods?limit=500") {
        lists += 1;
        return listOf([pod("web-0", "uid-0", String(10 + lists))], String(10 + lists));
      }
      return listOf([], "1");
    });

    const store = await connected();
    store.subscribe("conn-1", "pods", "");
    await settle();
    expect(lists).toBe(1);

    watchListener?.({
      streamId: "stream-1",
      kind: "ERROR",
      object: JSON.stringify({ kind: "Status", status: "Failure", reason: "Expired", code: 410, message: "too old" }),
    });
    await settle();

    expect(lists).toBe(2);
    expect(store.failureOf("conn-1", "pods", "")).toBeNull();
    expect(store.caches[cacheKeyOf("conn-1", "pods", "")]?.relists).toBe(1);
  });

  it("keeps the rows and records a hint when the watch was refused", async () => {
    requestMock.mockImplementation(async (_connId, request) =>
      request.path === "/api/v1/pods?limit=500" ? listOf([pod("web-0", "uid-0", "11")], "11") : listOf([], "1"),
    );
    watchStartMock.mockRejectedValue(
      new K8sHttpError(403, "/api/v1/pods?watch=true", JSON.stringify({ reason: "Forbidden" })),
    );

    const store = await connected();
    store.subscribe("conn-1", "pods", "");
    await settle();

    expect(store.rowsOf("conn-1", "pods", "")).toHaveLength(1);
    expect(store.failureOf("conn-1", "pods", "")).toBeNull();
    expect(store.watchFailureOf("conn-1", "pods", "")?.code).toBe("K8S-REQUEST-FAILED");
  });
});

describe("answers that arrive out of order", () => {
  it("drops a list answered after a reload had already replaced it", async () => {
    const releases: (() => void)[] = [];
    let call = 0;
    requestMock.mockImplementation(async (_connId, request) => {
      if (request.path !== "/api/v1/pods?limit=500") {
        return listOf([], "1");
      }
      call += 1;
      const mine = call;
      // The first list is held until the test lets it go, which is what "in flight" looks like.
      await new Promise<void>((resolve) => releases.push(resolve));
      return mine === 1
        ? listOf([pod("stale", "uid-stale", "11")], "11")
        : listOf([pod("fresh", "uid-fresh", "21")], "21");
    });

    const store = await connected();
    store.subscribe("conn-1", "pods", "");
    await settle();
    expect(releases).toHaveLength(1);

    store.refresh("conn-1", "pods", "");
    await settle();
    expect(releases).toHaveLength(2);

    // The reload answers first, then the stale one lands and must be ignored.
    releases[1]?.();
    await settle();
    releases[0]?.();
    await settle();

    expect(store.rowsOf("conn-1", "pods", "").map(nameOf)).toEqual(["fresh"]);
  });
});

describe("a list that cannot be read", () => {
  it("keeps the refusal on the cache as an envelope", async () => {
    requestMock.mockImplementation(async (_connId, request) => {
      if (request.path === "/api/v1/pods?limit=500") {
        return answer(403, { kind: "Status", reason: "Forbidden", code: 403, message: "pods is forbidden" });
      }
      return listOf([], "1");
    });

    const store = await connected();
    store.subscribe("conn-1", "pods", "");
    await settle();

    const failure = store.failureOf("conn-1", "pods", "");
    expect(failure?.code).toBe("K8S-REQUEST-FAILED");
    expect(failure?.envelope.messageParams.status).toBe("403");
    expect(store.isLoading("conn-1", "pods", "")).toBe(false);
  });
});

describe("metrics", () => {
  it("treats a 404 from the metrics API as a cluster without numbers", async () => {
    requestMock.mockImplementation(async (_connId, request) => {
      if (request.path.startsWith("/apis/metrics.k8s.io")) {
        return answer(404, { reason: "NotFound", message: "the server could not find the requested resource" });
      }
      return listOf([], "1");
    });

    const store = await connected();
    await store.loadMetrics("conn-1", "");

    expect(store.metricsOf("conn-1").available).toBe(false);
    expect(store.metricsOf("conn-1").failure).toBeNull();
    expect(store.usageOfPod("conn-1", "default", "web-0")).toBeNull();
  });

  it("reads pod and node usage in the units the columns sort by", async () => {
    requestMock.mockImplementation(async (_connId, request) => {
      if (request.path === "/apis/metrics.k8s.io/v1beta1/pods") {
        return answer(200, {
          items: [
            {
              metadata: { namespace: "default", name: "web-0" },
              containers: [{ usage: { cpu: "250m", memory: "64Mi" } }, { usage: { cpu: "1", memory: "1Gi" } }],
            },
          ],
        });
      }
      if (request.path === "/apis/metrics.k8s.io/v1beta1/nodes") {
        return answer(200, { items: [{ metadata: { name: "node-a" }, usage: { cpu: "1500m", memory: "2Gi" } }] });
      }
      return listOf([], "1");
    });

    const store = await connected();
    await store.loadMetrics("conn-1", "");

    expect(store.usageOfPod("conn-1", "default", "web-0")).toEqual({ cpu: 1250, mem: 64 * 1024 ** 2 + 1024 ** 3 });
    expect(store.usageOfNode("conn-1", "node-a")).toEqual({ cpu: 1500, mem: 2 * 1024 ** 3 });
    expect(store.metricsOf("conn-1").available).toBe(true);
  });
});

describe("writes", () => {
  it("replaces an edited manifest with an acknowledged PUT and relists what holds it", async () => {
    const configMap = {
      apiVersion: "v1",
      kind: "ConfigMap",
      metadata: { name: "settings", namespace: "default", uid: "uid-cm", resourceVersion: "7" },
      data: { mode: "old" },
    };
    requestMock.mockImplementation(async (_connId, request) =>
      request.path === "/api/v1/namespaces/default/configmaps?limit=500" ? listOf([configMap], "7") : listOf([], "1"),
    );

    const store = await connected();
    store.subscribe("conn-1", "configmaps", "default");
    await settle();

    const descriptor = descriptorOf("configmaps");
    if (!descriptor) {
      throw new Error("the configmaps descriptor is missing");
    }
    const body = JSON.stringify({ ...configMap, data: { mode: "new" } });

    expect(store.connectionIdOfObject(configMap)).toBe("conn-1");
    await store.replaceObject("conn-1", descriptor, configMap, body);
    await settle();

    const put = requestMock.mock.calls.find(([, request]) => request.method === "put")?.[1];
    expect(put).toEqual({
      method: "put",
      path: "/api/v1/namespaces/default/configmaps/settings",
      body,
      contentType: "application/json",
      acknowledged: true,
    });
    const lists = requestMock.mock.calls.filter(
      ([, request]) => request.path === "/api/v1/namespaces/default/configmaps?limit=500",
    );
    expect(lists).toHaveLength(2);
  });

  it("answers nothing for an object no list is holding", async () => {
    const store = await connected();
    expect(
      store.connectionIdOfObject({ metadata: { name: "gone", namespace: "default", uid: "uid-gone" } }),
    ).toBeNull();
  });

  it("acknowledges a delete and asks for the grace period a force delete means", async () => {
    requestMock.mockResolvedValue(answer(200, { kind: "Status", status: "Success" }));
    const store = await connected();
    const descriptor = descriptorOf("deployments");
    expect(descriptor).not.toBeNull();
    if (!descriptor) {
      return;
    }

    const target = { metadata: { name: "web", namespace: "default" } };
    await store.deleteObject("conn-1", descriptor, target, true);

    const sent = requestMock.mock.calls.at(-1)?.[1];
    expect(sent).toMatchObject({
      method: "delete",
      path: "/apis/apps/v1/namespaces/default/deployments/web?gracePeriodSeconds=0&propagationPolicy=Background",
      acknowledged: true,
    });
  });

  it("restarts a workload by stamping its pod template and scaling through the scale subresource", async () => {
    requestMock.mockResolvedValue(answer(200, {}));
    const store = await connected();
    const descriptor = descriptorOf("deployments");
    if (!descriptor) {
      throw new Error("the deployments descriptor is missing");
    }
    const target = { metadata: { name: "web", namespace: "default" }, spec: { replicas: 2 } };

    await store.restartWorkload("conn-1", descriptor, target);
    const restart = requestMock.mock.calls.at(-1)?.[1];
    expect(restart?.method).toBe("patch");
    expect(restart?.path).toBe("/apis/apps/v1/namespaces/default/deployments/web");
    expect(restart?.contentType).toBe("application/strategic-merge-patch+json");
    expect(JSON.parse(restart?.body ?? "{}")).toHaveProperty("spec.template.metadata.annotations");

    await store.scaleWorkload("conn-1", descriptor, target, 5);
    const scale = requestMock.mock.calls.at(-1)?.[1];
    expect(scale?.path).toBe("/apis/apps/v1/namespaces/default/deployments/web/scale");
    expect(scale?.body).toBe(JSON.stringify({ spec: { replicas: 5 } }));
    expect(scale?.acknowledged).toBe(true);
  });

  it("drains a node by cordoning it and evicting only what a drain may evict", async () => {
    const evictable = pod("web-0", "uid-0", "1");
    const daemonSet = {
      metadata: {
        name: "agent",
        namespace: "kube-system",
        uid: "uid-agent",
        ownerReferences: [{ kind: "DaemonSet", name: "agent", uid: "owner" }],
      },
      spec: { nodeName: "node-a" },
    };
    const staticPod = {
      metadata: {
        name: "etcd",
        namespace: "kube-system",
        uid: "uid-mirror",
        annotations: { "kubernetes.io/config.mirror": "x" },
      },
      spec: { nodeName: "node-a" },
    };

    requestMock.mockImplementation(async (_connId, request) => {
      if (request.path.startsWith("/api/v1/pods?fieldSelector=")) {
        return listOf([evictable, daemonSet, staticPod], "1");
      }
      if (request.path.endsWith("/eviction")) {
        return answer(201, { kind: "Status", status: "Success" });
      }
      return answer(200, {});
    });

    const store = await connected();
    const outcome = await store.drainNode("conn-1", "node-a");

    expect(outcome).toEqual({ evicted: 1, skipped: 2, failures: [] });
    const calls = requestMock.mock.calls.map(([, request]) => request);
    const cordon = calls.find((request) => request.path === "/api/v1/nodes/node-a");
    expect(cordon).toMatchObject({ method: "patch", body: JSON.stringify({ spec: { unschedulable: true } }) });
    const evictions = calls.filter((request) => request.path.endsWith("/eviction"));
    expect(evictions).toHaveLength(1);
    expect(evictions[0]?.path).toBe("/api/v1/namespaces/default/pods/web-0/eviction");
    expect(evictions[0]?.acknowledged).toBe(true);
  });
});

describe("kinds a cluster defines", () => {
  it("offers the kinds its definitions introduce, and forgets them when the connection closes", async () => {
    requestMock.mockImplementation(async (_connId, request) =>
      request.path === "/apis/apiextensions.k8s.io/v1/customresourcedefinitions?limit=500"
        ? listOf([widgetCrd], "1")
        : listOf([], "1"),
    );

    const store = await connected();
    expect(descriptorOf("crd:widgets.example.com")).not.toBeNull();

    await store.disconnectSession(session.id);
    expect(descriptorOf("crd:widgets.example.com")).toBeNull();
  });

  it("picks up a definition that arrives while the definitions are being followed", async () => {
    const store = await connected();
    store.subscribe("conn-1", "customresourcedefinitions", "");
    await settle();
    expect(descriptorOf("crd:widgets.example.com")).toBeNull();

    watchListener?.({
      streamId: "stream-1",
      kind: "ADDED",
      object: JSON.stringify({ ...widgetCrd, metadata: { uid: "uid-crd", name: "widgets.example.com" } }),
    });
    expect(descriptorOf("crd:widgets.example.com")).not.toBeNull();
  });
});

describe("the connection", () => {
  it("opens one connection per session and lets it go when the session closes", async () => {
    const store = await connected();
    await store.connect(session);
    expect(connectMock).toHaveBeenCalledTimes(1);

    await store.disconnectSession(session.id);
    expect(disconnectMock).toHaveBeenCalledWith("conn-1");
    expect(store.connectionOf(session.id)).toBeNull();
  });

  it("keeps the refusal where the pane can draw it", async () => {
    connectMock.mockRejectedValue(new K8sHttpError(503, "/version", "the API server is not there"));
    const store = useK8sStore();
    await store.connect(session);

    expect(store.connectionOf(session.id)?.phase).toBe("error");
    expect(store.connectionOf(session.id)?.failure?.code).toBe("K8S-REQUEST-FAILED");
  });

  it("opens a shell as an ordinary terminal named for the pod and container", async () => {
    execOpenMock.mockResolvedValue("term-9");
    const store = await connected();
    const terminals = useTerminalsStore();

    await store.openExec(session, "conn-1", pod("web-0", "uid-0", "1"), "main");

    const tab = terminals.tabs[0];
    expect(tab?.kind).toBe("terminal");
    expect(tab?.kind === "terminal" ? tab.handle : null).toBe("term-9");
    expect(tab?.kind === "terminal" ? tab.session.name : "").toBe("web-0/main");
  });
});

describe("cache keys", () => {
  it("scopes a cluster-wide kind to no namespace, whatever the picker says", () => {
    const nodes = descriptorOf("nodes");
    const pods = descriptorOf("pods");
    expect(nodes).not.toBeNull();
    expect(pods).not.toBeNull();
    if (!nodes || !pods) {
      return;
    }
    expect(scopedNamespace(nodes, "default")).toBe("");
    expect(scopedNamespace(pods, "default")).toBe("default");
    expect(cacheKeyOf("conn-1", "nodes", "")).toBe("conn-1::nodes::");
  });
});
