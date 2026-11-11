/**
 * A cluster connection and the live lists it feeds.
 *
 * The shape of the problem: a list request answers a page and a `resourceVersion`, and a watch on the
 * same collection answers every change after that version. A list is only ever half the answer, and a
 * view that drew the page and stopped would be a screenshot. Every list this store hands out is
 * therefore list-then-watch, keyed by connection, kind and namespace.
 *
 * Three rules the cache follows, all of them learned from what goes wrong without them:
 *
 * 1. **A cache is shared by everyone looking at it, and closed by the last one.** Two places can ask
 *    for the same list — a list and a detail view, a poll and a person — and both should cost one
 *    subscription. The refcount decides when the watch is let go, because a watch stopped under
 *    somebody's list is a list that silently stops being live.
 * 2. **A late answer is thrown away, not applied.** Reloading while a list is in flight leaves two
 *    answers arriving out of order; the generation counter says which one the view is still waiting
 *    for, and the other is dropped — including the watch it started, which is stopped again rather
 *    than left running with nothing pointing at it.
 * 3. **An expired watch relists; it does not become an error.** The API server answers an old
 *    `resourceVersion` with a 410, which is ordinary housekeeping rather than a fault: the answer is
 *    to read the list again and carry on, which is what `relist` does.
 *
 * Nothing here translates: a failure is an envelope, and the pane renders it with the same
 * `describeShellError` every other page uses.
 */
import { defineStore } from "pinia";
import { ref } from "vue";

import {
  deletePath,
  drainPlan,
  evictionBody,
  evictionPath,
  podsOnNodePath,
  restartContentType,
  restartPatch,
  restartPath,
  scaleContentType,
  scalePatch,
  unschedulablePatch,
} from "@/components/k8s/actions";
import { parseNodeMetrics, parsePodMetrics, podUsageKey, totalUsage } from "@/components/k8s/metrics";
import {
  age,
  allocatableOf,
  arrayAt,
  asObject,
  idOf,
  nameOf,
  namespaceOf,
  nodeReady,
  nodeRoles,
  numberAt,
  podRequests,
  podStatus,
  textAt,
  type K8sObject,
  type Usage,
} from "@/components/k8s/objects";
import {
  CRD_KEY,
  clearCustomResourceDescriptors,
  collectionPath,
  customResourceDescriptor,
  descriptorOf,
  listPathOf,
  selfPathOf,
  setCustomResourceDescriptors,
  watchPathOf,
  type ResourceDescriptor,
} from "@/components/k8s/resources";
import { backend } from "@/lib/backend/api";
import { ShellError, type ErrorEnvelope, asShellError } from "@/lib/backend/errors";
import { messageKeyFor } from "@/lib/backend/errorCodes";
import { useTerminalsStore } from "@/stores/terminals";
import type { K8sRequest, K8sWatchEvent } from "@/types/k8s";
import type { Session } from "@/types/session";

// ── errors ──────────────────────────────────────────────────────

/** The excerpt of a body an error carries: enough to recognise the Status, short enough to read. */
function excerpt(body: string): string {
  const flat = body.replace(/\s+/g, " ").trim();
  return flat.length > 240 ? `${flat.slice(0, 240)}…` : flat;
}

/**
 * A request the API server answered with a status this build does not accept.
 *
 * Built here rather than carried back from Rust because a non-2xx status is not a rejection: the
 * generic request command succeeds and the status is part of its answer. The envelope is the same
 * shape the shell would have produced, so the pane renders it the same way.
 */
export class K8sHttpError extends ShellError {
  readonly status: number;

  constructor(status: number, path: string, body: string) {
    const envelope: ErrorEnvelope = {
      version: 1,
      code: "K8S-REQUEST-FAILED",
      messageKey: messageKeyFor("K8S-REQUEST-FAILED"),
      messageParams: { status: String(status) },
      origin: { subsystem: "k8s" },
      outcome: "rejected",
      detail: path === "" ? excerpt(body) : `${path}: ${excerpt(body)}`,
    };
    super(envelope);
    this.status = status;
    // Keeps `instanceof` reliable when the class is compiled down to a function.
    Object.setPrototypeOf(this, K8sHttpError.prototype);
  }
}

/** Whether an answer said "this cluster has no such API", which is how a missing metrics server reads. */
export function isMissingApi(error: unknown, status = 404): boolean {
  return error instanceof K8sHttpError && error.status === status;
}

// ── connection, cache and metrics shapes ────────────────────────

export interface K8sConnection {
  /** The session this connection belongs to. */
  readonly sessionId: string;
  /** The backend's handle, empty until the connection is up. */
  readonly id: string;
  readonly phase: "connecting" | "connected" | "error";
  readonly failure: ShellError | null;
  readonly context: string;
  readonly version: string;
  /** The namespaces the cluster let this connection see. Empty where the list was refused. */
  readonly namespaces: readonly string[];
}

/**
 * One collection, as it is being followed.
 *
 * `items` is a `Map` held by identity rather than replaced on every event: a busy namespace sends
 * hundreds of changes a minute, and copying the whole list for each one is how a watch turns into a
 * slideshow. What changes on an event is `revision`, which is what a view watches instead.
 */
export interface ResourceCache {
  readonly connId: string;
  readonly resourceKey: string;
  /** The namespace this cache is scoped to; empty for a cluster-wide kind or an all-namespaces list. */
  readonly namespace: string;
  readonly items: ReadonlyMap<string, K8sObject>;
  /** Bumped whenever `items` or `resourceVersion` changes. Views depend on this, not on the map. */
  readonly revision: number;
  /** Bumped on every (re)load; an answer for an older generation is dropped. */
  readonly generation: number;
  readonly refCount: number;
  readonly resourceVersion: string;
  readonly streamId: string | null;
  readonly loading: boolean;
  /** Why the last list failed. A list that failed shows this instead of rows. */
  readonly failure: ShellError | null;
  /** Why the watch is not up. The rows are still usable, so this is a hint rather than a failure. */
  readonly watchFailure: ShellError | null;
  /** How many times this cache had to start over because the API server expired the watch. */
  readonly relists: number;
}

/** What the metrics API says is in use, per connection: two clusters can name a pod the same way. */
export interface MetricsState {
  readonly pods: Readonly<Record<string, Usage>>;
  readonly nodes: Readonly<Record<string, Usage>>;
  /** False where the cluster has no metrics API at all, which makes every reading `—`. */
  readonly available: boolean;
  /** A metrics read that failed for another reason — a permission, a timeout. */
  readonly failure: ShellError | null;
}

export interface NodeSummary {
  readonly name: string;
  readonly ready: boolean;
  readonly roles: string;
  readonly allocatable: Usage;
}

export interface PodCounts {
  readonly total: number;
  readonly running: number;
  readonly pending: number;
  readonly succeeded: number;
  readonly failed: number;
  readonly unknown: number;
  readonly notReady: number;
}

/** What the overview page reads, gathered in one pass so its numbers agree with each other. */
export interface ClusterOverview {
  readonly version: string;
  readonly nodes: readonly NodeSummary[];
  readonly pods: PodCounts;
  /** The CPU and memory every pod on the cluster asks for. */
  readonly requests: Usage;
  /** How many of each kind exist, by resource key; `null` where the count could not be read. */
  readonly counts: Readonly<Record<string, number | null>>;
  readonly updatedAt: number;
}

/** The kinds the overview counts. Everything else is reached through the tree. */
const OVERVIEW_COUNTS: readonly string[] = [
  "deployments",
  "statefulsets",
  "daemonsets",
  "jobs",
  "cronjobs",
  "services",
  "ingresses",
  "configmaps",
  "secrets",
  "persistentvolumeclaims",
  "persistentvolumes",
  "namespaces",
];

const EMPTY_METRICS: MetricsState = { pods: {}, nodes: {}, available: true, failure: null };

/** A cache key, as the contract spells it: `connId::resourceKey::namespace`. */
export function cacheKeyOf(connId: string, resourceKey: string, namespace: string): string {
  return `${connId}::${resourceKey}::${namespace}`;
}

/** The namespace a kind is scoped to: cluster-wide kinds ignore what the picker says. */
export function scopedNamespace(desc: ResourceDescriptor, namespace: string): string {
  return desc.namespaced ? namespace : "";
}

export interface DrainResult {
  readonly evicted: number;
  readonly skipped: number;
  /** One line per pod whose eviction the API server refused. */
  readonly failures: readonly string[];
}

function parseJson(body: string): unknown {
  if (body.trim() === "") {
    return null;
  }
  try {
    return JSON.parse(body) as unknown;
  } catch {
    return null;
  }
}

/** The objects of a list answer, dropping anything that is not one. */
function itemsOf(body: unknown): readonly K8sObject[] {
  return arrayAt(body, "items")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null);
}

export const useK8sStore = defineStore("k8s", () => {
  const connections = ref<Record<string, K8sConnection>>({});
  const caches = ref<Record<string, ResourceCache>>({});
  const metrics = ref<Record<string, MetricsState>>({});
  const overviews = ref<Record<string, ClusterOverview>>({});
  const overviewFailures = ref<Record<string, ShellError>>({});
  /** False when the watch channel could not be opened: lists still read, they just do not follow. */
  const liveUpdates = ref(true);

  /** Which cache a watch belongs to. A stream id is the only thing an event carries. */
  const watchIndex = new Map<string, string>();
  let stopWatchChannel: (() => void) | null = null;
  let watchChannel: Promise<void> | null = null;

  // ── plumbing ──────────────────────────────────────────────────

  function withConnection(sessionId: string, next: K8sConnection): void {
    connections.value = { ...connections.value, [sessionId]: next };
  }

  function patchConnection(sessionId: string, fields: Partial<K8sConnection>): void {
    const current = connections.value[sessionId];
    if (current) {
      withConnection(sessionId, { ...current, ...fields });
    }
  }

  /**
   * Replaces one cache entry.
   *
   * The whole record is replaced rather than mutated so a view holding a computed over it
   * re-evaluates: with `ref` over an object, assigning a field is reactive but mutating a `Map`
   * inside it is not, and this is the one place that decision is made.
   */
  function patch(key: string, fields: Partial<ResourceCache>): void {
    const current = caches.value[key];
    if (!current) {
      return;
    }
    caches.value = { ...caches.value, [key]: { ...current, ...fields } };
  }

  function stillCurrent(key: string, generation: number): boolean {
    return caches.value[key]?.generation === generation;
  }

  async function stopStreamById(streamId: string): Promise<void> {
    watchIndex.delete(streamId);
    try {
      await backend.k8s.streamStop(streamId);
    } catch {
      // A stream the backend has already forgotten needs no second word.
    }
  }

  async function stopStream(key: string): Promise<void> {
    const streamId = caches.value[key]?.streamId ?? null;
    if (streamId === null) {
      return;
    }
    patch(key, { streamId: null });
    await stopStreamById(streamId);
  }

  /**
   * Sends one request and answers the body the server sent.
   *
   * A non-2xx status throws {@link K8sHttpError}, so a caller can tell a missing metrics API (404)
   * from a refused list (403) without reading the body itself.
   */
  async function request(connId: string, req: K8sRequest): Promise<unknown> {
    const response = await backend.k8s.request(connId, req);
    if (response.status < 200 || response.status >= 300) {
      throw new K8sHttpError(response.status, req.path, response.body);
    }
    return parseJson(response.body);
  }

  // ── the connection ────────────────────────────────────────────

  /**
   * Opens the cluster behind a session, once.
   *
   * A second call for the same session answers nothing rather than a second connection: the pane
   * connects on mount, and a tab that comes on screen again must not leave the first connection
   * behind. `reconnect` is the way out of a failed one.
   */
  async function connect(session: Session): Promise<void> {
    if (connections.value[session.id] !== undefined) {
      return;
    }
    const context = session.kubernetes?.context ?? "";
    withConnection(session.id, {
      sessionId: session.id,
      id: "",
      phase: "connecting",
      failure: null,
      context,
      version: "",
      namespaces: [],
    });
    let id: string;
    try {
      id = await backend.k8s.connect(session.id);
    } catch (error) {
      patchConnection(session.id, { phase: "error", failure: asShellError(error, "k8s_connect") });
      return;
    }
    patchConnection(session.id, { id, phase: "connected" });
    await listen();
    const namespaces = await readNamespaces(id);
    patchConnection(session.id, { namespaces });
    await readCustomResources(id);
  }

  /** Closes a failed or stale connection and opens it again, which is what the retry button does. */
  async function reconnect(session: Session): Promise<void> {
    await disconnectSession(session.id);
    await connect(session);
  }

  /**
   * Lets go of a session's connection and everything following it.
   *
   * The streams go first: a watch outliving its connection is an event with nowhere to land, and
   * the backend would be left holding a channel nobody can name.
   */
  async function disconnectSession(sessionId: string): Promise<void> {
    const connection = connections.value[sessionId];
    if (!connection) {
      return;
    }
    for (const [key, cache] of Object.entries(caches.value)) {
      if (cache.connId !== connection.id) {
        continue;
      }
      if (cache.streamId !== null) {
        await stopStreamById(cache.streamId);
      }
      const next = { ...caches.value };
      delete next[key];
      caches.value = next;
    }
    const remaining = { ...connections.value };
    delete remaining[sessionId];
    connections.value = remaining;
    if (connection.id !== "") {
      // The tree may still be drawing this connection's custom kinds; their paths go with it.
      clearCustomResourceDescriptors(connection.id);
      try {
        await backend.k8s.disconnect(connection.id);
      } catch {
        // A backend that has already dropped the connection has nothing left to say about it.
      }
    }
  }

  function connectionIdOf(sessionId: string): string | null {
    const connection = connections.value[sessionId];
    return connection !== undefined && connection.id !== "" ? connection.id : null;
  }

  function connectionOf(sessionId: string): K8sConnection | null {
    return connections.value[sessionId] ?? null;
  }

  /** Opens the watch channel, once for the whole application. */
  async function listen(): Promise<void> {
    if (watchChannel !== null) {
      await watchChannel;
      return;
    }
    watchChannel = (async () => {
      try {
        stopWatchChannel = await backend.k8s.onWatch(deliverWatch);
      } catch {
        // Without a watch channel every list is a page that does not move. That is a degradation
        // worth saying, not a failure worth stopping the workspace for.
        liveUpdates.value = false;
      }
    })();
    await watchChannel;
  }

  /** The unsubscribe for the watch channel, for a caller that is shutting the workspace down. */
  function release(): void {
    stopWatchChannel?.();
    stopWatchChannel = null;
    watchChannel = null;
  }

  /** The namespaces the connection may list into, for the picker's suggestions. */
  async function readNamespaces(connId: string): Promise<readonly string[]> {
    try {
      const body = await request(connId, { method: "get", path: "/api/v1/namespaces?limit=500" });
      return arrayAt(body, "items")
        .map((entry) => asObject(entry))
        .filter((entry): entry is K8sObject => entry !== null)
        .map((namespace) => nameOf(namespace))
        .filter((name) => name !== "")
        .sort();
    } catch {
      // A connection that may only see its own namespace still works: an empty list leaves the
      // picker with the all-namespaces reading and whatever the session named.
      return [];
    }
  }

  /**
   * Reads the defined kinds a connection offers.
   *
   * A refusal is not reported: a cluster that will not list CRDs still browses every built-in kind,
   * and the tree simply has no custom group. An API server without apiextensions answers 404, which
   * is the same fact.
   */
  async function readCustomResources(connId: string): Promise<void> {
    const desc = descriptorOf(CRD_KEY);
    if (!desc) {
      return;
    }
    try {
      const body = await request(connId, { method: "get", path: listPathOf(desc, "") });
      // The tab may have closed while the answer was on its way; a registration for a connection
      // that no longer exists would keep its kinds in the tree with nowhere to list them.
      if (!Object.values(connections.value).some((entry) => entry.id === connId)) {
        return;
      }
      publishCustomResources(connId, itemsOf(body));
    } catch {
      // No definitions to offer; every other list still works.
    }
  }

  /** Turns the definitions a connection sent into the descriptors the tree draws. */
  function publishCustomResources(connId: string, items: Iterable<K8sObject>): void {
    const descriptors: ResourceDescriptor[] = [];
    for (const item of items) {
      const descriptor = customResourceDescriptor(item);
      if (descriptor !== null) {
        descriptors.push(descriptor);
      }
    }
    setCustomResourceDescriptors(connId, descriptors);
  }

  // ── lists ─────────────────────────────────────────────────────

  /**
   * Follows one collection, adding this caller to its refcount.
   *
   * Only the first caller lists and watches; a second one joins what is already there, which is what
   * lets a list and a detail view share one subscription.
   */
  function subscribe(connId: string, resourceKey: string, namespace: string): void {
    const desc = descriptorOf(resourceKey);
    if (!desc || connId === "") {
      return;
    }
    const scoped = scopedNamespace(desc, namespace);
    const key = cacheKeyOf(connId, resourceKey, scoped);
    const existing = caches.value[key];
    if (existing) {
      patch(key, { refCount: existing.refCount + 1 });
      return;
    }

    const created: ResourceCache = {
      connId,
      resourceKey,
      namespace: scoped,
      items: new Map(),
      revision: 0,
      generation: 1,
      refCount: 1,
      resourceVersion: "",
      streamId: null,
      loading: true,
      failure: null,
      watchFailure: null,
      relists: 0,
    };
    caches.value = { ...caches.value, [key]: created };
    void loadList(key);
  }

  /** Lets one caller go; the last one out stops the watch and takes the cache with it. */
  function unsubscribe(connId: string, resourceKey: string, namespace: string): void {
    const desc = descriptorOf(resourceKey);
    if (!desc) {
      return;
    }
    const key = cacheKeyOf(connId, resourceKey, scopedNamespace(desc, namespace));
    const cache = caches.value[key];
    if (!cache) {
      return;
    }
    if (cache.refCount > 1) {
      patch(key, { refCount: cache.refCount - 1 });
      return;
    }
    const streamId = cache.streamId;
    const next = { ...caches.value };
    delete next[key];
    caches.value = next;
    if (streamId !== null) {
      void stopStreamById(streamId);
    }
  }

  /** Reads the collection again, keeping the rows on screen while the answer is on its way. */
  function refresh(connId: string, resourceKey: string, namespace: string): void {
    const desc = descriptorOf(resourceKey);
    if (!desc) {
      return;
    }
    const key = cacheKeyOf(connId, resourceKey, scopedNamespace(desc, namespace));
    const cache = caches.value[key];
    if (!cache) {
      return;
    }
    patch(key, { generation: cache.generation + 1 });
    void loadList(key);
  }

  /**
   * Starts the watch over from a fresh list.
   *
   * The rows stay: an expired `resourceVersion` says the watch is stale, not that the objects left.
   */
  async function relist(key: string): Promise<void> {
    const cache = caches.value[key];
    if (!cache) {
      return;
    }
    patch(key, { generation: cache.generation + 1, relists: cache.relists + 1 });
    await loadList(key);
  }

  async function loadList(key: string): Promise<void> {
    const cache = caches.value[key];
    const desc = cache ? descriptorOf(cache.resourceKey) : null;
    if (!cache || !desc) {
      return;
    }
    const generation = cache.generation;
    patch(key, { loading: true, failure: null });
    await stopStream(key);
    if (!stillCurrent(key, generation)) {
      return;
    }

    let body: unknown;
    try {
      body = await request(cache.connId, { method: "get", path: listPathOf(desc, cache.namespace) });
    } catch (error) {
      if (!stillCurrent(key, generation)) {
        return;
      }
      patch(key, { loading: false, failure: asShellError(error, "k8s_request") });
      return;
    }
    if (!stillCurrent(key, generation)) {
      return;
    }

    const items = new Map<string, K8sObject>();
    for (const entry of arrayAt(body, "items")) {
      const object = asObject(entry);
      if (!object) {
        continue;
      }
      const id = idOf(object);
      if (id !== "") {
        items.set(id, object);
      }
    }
    patch(key, {
      items,
      revision: 0,
      resourceVersion: textAt(body, "metadata", "resourceVersion"),
      loading: false,
      failure: null,
    });
    if (cache.resourceKey === CRD_KEY) {
      publishCustomResources(cache.connId, items.values());
    }
    await startWatch(key, generation);
  }

  async function startWatch(key: string, generation: number): Promise<void> {
    const cache = caches.value[key];
    const desc = cache ? descriptorOf(cache.resourceKey) : null;
    if (!cache || !desc) {
      return;
    }
    try {
      const streamId = await backend.k8s.watchStart(
        cache.connId,
        watchPathOf(desc, cache.namespace, cache.resourceVersion),
      );
      // The caller may have gone while the watch was being opened: the stream is this store's
      // alone, so it is closed here rather than left for nobody to collect.
      if (!stillCurrent(key, generation)) {
        await stopStreamById(streamId);
        return;
      }
      watchIndex.set(streamId, key);
      patch(key, { streamId, watchFailure: null });
    } catch (error) {
      if (!stillCurrent(key, generation)) {
        return;
      }
      patch(key, { watchFailure: asShellError(error, "k8s_watch_start") });
    }
  }

  /**
   * One watch event, applied to the cache its stream belongs to.
   *
   * The API server sends the object as a string, exactly as it wrote it on the wire; that is parsed
   * here and nowhere else, so a malformed event is dropped rather than spread through the view.
   */
  function deliverWatch(event: K8sWatchEvent): void {
    const key = watchIndex.get(event.streamId);
    if (key === undefined) {
      return;
    }
    const cache = caches.value[key];
    if (!cache) {
      return;
    }
    const object = asObject(parseJson(event.object));

    if (event.kind === "BOOKMARK") {
      const version = object ? textAt(object, "metadata", "resourceVersion") : "";
      if (version !== "") {
        patch(key, { resourceVersion: version });
      }
      return;
    }
    if (event.kind === "ERROR") {
      handleWatchError(key, object);
      return;
    }
    if (!object) {
      return;
    }
    const id = idOf(object);
    if (id === "") {
      return;
    }
    const version = textAt(object, "metadata", "resourceVersion");
    if (event.kind === "ADDED" || event.kind === "MODIFIED") {
      const items = new Map(cache.items);
      items.set(id, object);
      patch(key, { items, revision: cache.revision + 1, resourceVersion: version || cache.resourceVersion });
      if (cache.resourceKey === CRD_KEY) {
        // A definition somebody just created is a new kind to draw; nothing else announces it.
        publishCustomResources(cache.connId, items.values());
      }
      return;
    }
    if (event.kind === "DELETED") {
      const items = new Map(cache.items);
      items.delete(id);
      patch(key, { items, revision: cache.revision + 1, resourceVersion: version || cache.resourceVersion });
      if (cache.resourceKey === CRD_KEY) {
        publishCustomResources(cache.connId, items.values());
      }
    }
  }

  /**
   * A watch the API server ended.
   *
   * `410 Gone` and the `Expired` reason both mean "ask again from scratch". Anything else — a
   * forbidden watch, a bad path — is reported where the list draws its live hint, because the rows
   * that were already read are still true.
   */
  function handleWatchError(key: string, status: K8sObject | null): void {
    const code = status ? (numberAt(status, "code") ?? 0) : 0;
    const reason = status ? textAt(status, "reason") : "";
    if (code === 410 || reason === "Expired" || reason === "Gone") {
      void relist(key);
      return;
    }
    const message = status ? textAt(status, "message") : "";
    patch(key, { watchFailure: new K8sHttpError(code === 0 ? 500 : code, "", message) });
  }

  // ── what a view reads ─────────────────────────────────────────

  function cacheOf(connId: string, resourceKey: string, namespace: string): ResourceCache | null {
    const desc = descriptorOf(resourceKey);
    if (!desc) {
      return null;
    }
    return caches.value[cacheKeyOf(connId, resourceKey, scopedNamespace(desc, namespace))] ?? null;
  }

  /** The rows of a collection, newest answer only. Sorting is the table's business. */
  function rowsOf(connId: string, resourceKey: string, namespace: string): readonly K8sObject[] {
    const cache = cacheOf(connId, resourceKey, namespace);
    return cache ? [...cache.items.values()] : [];
  }

  function isLoading(connId: string, resourceKey: string, namespace: string): boolean {
    return cacheOf(connId, resourceKey, namespace)?.loading ?? false;
  }

  function failureOf(connId: string, resourceKey: string, namespace: string): ShellError | null {
    return cacheOf(connId, resourceKey, namespace)?.failure ?? null;
  }

  function watchFailureOf(connId: string, resourceKey: string, namespace: string): ShellError | null {
    return cacheOf(connId, resourceKey, namespace)?.watchFailure ?? null;
  }

  // ── metrics ───────────────────────────────────────────────────

  function podMetricsPath(namespace: string): string {
    return namespace === ""
      ? "/apis/metrics.k8s.io/v1beta1/pods"
      : `/apis/metrics.k8s.io/v1beta1/namespaces/${encodeURIComponent(namespace)}/pods`;
  }

  const NODE_METRICS_PATH = "/apis/metrics.k8s.io/v1beta1/nodes";

  function metricsOf(connId: string): MetricsState {
    return metrics.value[connId] ?? EMPTY_METRICS;
  }

  /**
   * Reads what the metrics server says is in use.
   *
   * A 404 is the cluster saying "no metrics server", which is a fact about the cluster: every
   * reading becomes `—` and nothing is reported as broken. Any other refusal is recorded, because a
   * 403 is a permission somebody can fix.
   */
  async function loadMetrics(connId: string, namespace: string): Promise<void> {
    const current = metricsOf(connId);
    let pods = current.pods;
    let nodes = current.nodes;
    let available = current.available;
    let failure: ShellError | null = null;

    try {
      pods = parsePodMetrics(await request(connId, { method: "get", path: podMetricsPath(namespace) }));
      available = true;
    } catch (error) {
      if (isMissingApi(error)) {
        pods = {};
        available = false;
      } else {
        failure = asShellError(error, "k8s_metrics");
      }
    }

    try {
      nodes = parseNodeMetrics(await request(connId, { method: "get", path: NODE_METRICS_PATH }));
    } catch (error) {
      if (isMissingApi(error)) {
        nodes = {};
      } else {
        failure = failure ?? asShellError(error, "k8s_metrics");
      }
    }

    metrics.value = { ...metrics.value, [connId]: { pods, nodes, available, failure } };
  }

  /** What is in use on one pod, or `null` where the cluster has no numbers for it. */
  function usageOfPod(connId: string, namespace: string, name: string): Usage | null {
    return metricsOf(connId).pods[podUsageKey(namespace, name)] ?? null;
  }

  function usageOfNode(connId: string, name: string): Usage | null {
    return metricsOf(connId).nodes[name] ?? null;
  }

  /** The CPU and memory in use across the nodes, for the overview's meters. */
  function clusterUsage(connId: string): Usage {
    return totalUsage(metricsOf(connId).nodes);
  }

  // ── the overview ──────────────────────────────────────────────

  /** The items of a page, or `null` when the call failed — a count that cannot be read stays blank. */
  async function itemsAt(
    connId: string,
    path: string,
  ): Promise<{ readonly items: readonly K8sObject[]; readonly total: number } | null> {
    try {
      const body = await request(connId, { method: "get", path });
      const items = arrayAt(body, "items")
        .map((entry) => asObject(entry))
        .filter((entry): entry is K8sObject => entry !== null);
      const remaining = numberAt(body, "metadata", "remainingItemCount") ?? 0;
      return { items, total: items.length + remaining };
    } catch {
      return null;
    }
  }

  /**
   * Reads everything the overview draws, in one pass.
   *
   * One pass rather than one per card so the numbers agree: a pod counted as running by the tile and
   * as pending by a list would be two answers about one moment.
   */
  async function loadOverview(connId: string): Promise<void> {
    let firstError: unknown = null;
    const track = (error: unknown): void => {
      firstError = firstError ?? error;
    };

    const versionBody = await request(connId, { method: "get", path: "/version" }).catch((error: unknown) => {
      track(error);
      return null;
    });
    const version = textAt(versionBody, "gitVersion");

    const nodePage = await itemsAt(connId, "/api/v1/nodes");
    const nodes: NodeSummary[] = (nodePage?.items ?? []).map((node) => ({
      name: nameOf(node),
      ready: nodeReady(node),
      roles: nodeRoles(node),
      allocatable: allocatableOf(node),
    }));

    const podPage = await itemsAt(connId, "/api/v1/pods?limit=500");
    const counts = {
      total: 0,
      running: 0,
      pending: 0,
      succeeded: 0,
      failed: 0,
      unknown: 0,
      notReady: 0,
    };
    let cpu = 0;
    let mem = 0;
    for (const pod of podPage?.items ?? []) {
      const phase = textAt(pod, "status", "phase");
      if (phase === "Running") {
        counts.running += 1;
      } else if (phase === "Pending") {
        counts.pending += 1;
      } else if (phase === "Succeeded") {
        counts.succeeded += 1;
      } else if (phase === "Failed") {
        counts.failed += 1;
      } else {
        counts.unknown += 1;
      }
      const ready = arrayAt(pod, "status", "conditions")
        .map((entry) => asObject(entry))
        .find((condition): condition is K8sObject => condition !== null && textAt(condition, "type") === "Ready");
      if (!ready || textAt(ready, "status") !== "True") {
        counts.notReady += 1;
      }
      const requests = podRequests(pod);
      cpu += requests.cpu;
      mem += requests.mem;
    }
    counts.total = podPage?.total ?? 0;

    const countsByKey: Record<string, number | null> = {};
    for (const key of OVERVIEW_COUNTS) {
      const desc = descriptorOf(key);
      const page = desc ? await itemsAt(connId, listPathOf(desc, "")) : null;
      countsByKey[key] = page?.total ?? null;
    }

    overviews.value = {
      ...overviews.value,
      [connId]: {
        version,
        nodes,
        pods: counts,
        requests: { cpu, mem },
        counts: countsByKey,
        updatedAt: Date.now(),
      },
    };

    if (nodePage === null && podPage === null && version === "" && firstError !== null) {
      overviewFailures.value = { ...overviewFailures.value, [connId]: asShellError(firstError, "k8s_overview") };
      return;
    }
    const nextFailures = { ...overviewFailures.value };
    delete nextFailures[connId];
    overviewFailures.value = nextFailures;

    const connection = Object.values(connections.value).find((entry) => entry.id === connId);
    if (connection !== undefined && connection.version !== version) {
      patchConnection(connection.sessionId, { version });
    }
  }

  /** How long ago the overview was read, in the short shape the header uses. */
  function overviewAge(connId: string): string {
    const overview = overviews.value[connId];
    return overview ? age(new Date(overview.updatedAt).toISOString()) : "—";
  }

  // ── writes ────────────────────────────────────────────────────

  /**
   * The connection an object on screen belongs to.
   *
   * The detail drawer is handed the object and its descriptor, not the connection — the pane's props
   * are fixed — and the cache that is following the object is where the connection id still is. A
   * row that is on screen is always in one: the drawer draws what the list drew.
   */
  function connectionIdOfObject(obj: K8sObject): string | null {
    const id = idOf(obj);
    for (const cache of Object.values(caches.value)) {
      if (cache.items.has(id)) {
        return cache.connId;
      }
    }
    return null;
  }

  /**
   * Replaces one object with the document a person edited.
   *
   * A PUT, not a merge patch: the editor shows the whole object, so the whole object is what is
   * written back. `acknowledged` travels because writing to a cluster is policy-gated exactly as a
   * delete is; the person answered by pressing Save on a document they could read.
   *
   * Every cache holding the object is relisted afterwards: the answer is what the server accepted,
   * and a tab whose watch is slow should not keep showing the draft it replaced.
   */
  async function replaceObject(connId: string, desc: ResourceDescriptor, obj: K8sObject, body: string): Promise<void> {
    await request(connId, {
      method: "put",
      path: selfPathOf(desc, obj),
      body,
      contentType: "application/json",
      acknowledged: true,
    });
    const id = idOf(obj);
    for (const cache of Object.values(caches.value)) {
      if (cache.connId === connId && cache.items.has(id)) {
        refresh(connId, cache.resourceKey, cache.namespace);
      }
    }
  }

  /**
   * Deletes one object.
   *
   * `acknowledged` travels because this is a destructive request: the backend's policy gate refuses
   * one that does not carry the person's answer, and the answer here is the confirmation the pane
   * put in front of them.
   */
  async function deleteObject(connId: string, desc: ResourceDescriptor, obj: K8sObject, force: boolean): Promise<void> {
    await request(connId, {
      method: "delete",
      path: deletePath(selfPathOf(desc, obj), force),
      acknowledged: true,
    });
  }

  /** Restarts a workload by stamping its pod template, which is what makes its controller roll. */
  async function restartWorkload(connId: string, desc: ResourceDescriptor, obj: K8sObject): Promise<void> {
    const path = restartPath(desc, obj);
    if (path === null) {
      return;
    }
    await request(connId, {
      method: "patch",
      path,
      body: restartPatch(new Date().toISOString()),
      contentType: restartContentType(),
      acknowledged: true,
    });
  }

  async function scaleWorkload(
    connId: string,
    desc: ResourceDescriptor,
    obj: K8sObject,
    replicas: number,
  ): Promise<void> {
    await request(connId, {
      method: "patch",
      path: `${selfPathOf(desc, obj)}/scale`,
      body: scalePatch(replicas),
      contentType: scaleContentType(),
      acknowledged: true,
    });
  }

  /** Cordon (true) or uncordon (false) one node. */
  async function setUnschedulable(connId: string, name: string, unschedulable: boolean): Promise<void> {
    await request(connId, {
      method: "patch",
      path: `/api/v1/nodes/${encodeURIComponent(name)}`,
      body: unschedulablePatch(unschedulable),
      contentType: "application/merge-patch+json",
      acknowledged: true,
    });
  }

  /**
   * Drains one node: cordon it, then ask the API server to evict what is on it.
   *
   * The eviction endpoint is what honours a disruption budget; a plain delete would not, and a drain
   * that ignored budgets would take services down one node at a time. Pods the drain must leave
   * (DaemonSets' own, static mirror pods) are counted rather than failed.
   */
  async function drainNode(connId: string, name: string): Promise<DrainResult> {
    await setUnschedulable(connId, name, true);
    const body = await request(connId, { method: "get", path: podsOnNodePath(name) });
    const pods = arrayAt(body, "items")
      .map((entry) => asObject(entry))
      .filter((entry): entry is K8sObject => entry !== null);
    const plan = drainPlan(pods, name);
    const failures: string[] = [];
    let evicted = 0;
    for (const pod of plan.evict) {
      const namespace = namespaceOf(pod);
      const podName = nameOf(pod);
      try {
        await request(connId, {
          method: "post",
          path: evictionPath(namespace, podName),
          body: evictionBody(namespace, podName),
          contentType: "application/json",
          acknowledged: true,
        });
        evicted += 1;
      } catch (error) {
        failures.push(`${namespace}/${podName}: ${asShellError(error, "k8s_eviction").code}`);
      }
    }
    return {
      evicted,
      skipped: plan.keep.filter((entry) => entry.why !== "elsewhere").length,
      failures,
    };
  }

  /**
   * Opens a shell inside a pod, as an ordinary terminal tab.
   *
   * The handle is a terminal like any other once it is open, which is what lets resize, search and
   * close work without a second code path — and why the session it is drawn under is named for the
   * pod and container rather than for the cluster.
   */
  async function openExec(session: Session, connId: string, pod: K8sObject, container: string): Promise<void> {
    const podName = nameOf(pod);
    const handle = await backend.k8s.execOpen(connId, {
      namespace: namespaceOf(pod),
      pod: podName,
      container,
    });
    useTerminalsStore().openHandle({ ...session, name: `${podName}/${container}` }, handle);
  }

  /**
   * Creates one object from a manifest the person edited.
   *
   * `application/yaml` because the editor is a document, not a form: the server parses it and its
   * own schema is what decides. The list is relisted rather than waited on — the watch would bring
   * the object in eventually, and "eventually" reads as a create that did nothing.
   */
  async function createObject(
    connId: string,
    desc: ResourceDescriptor,
    namespace: string,
    manifest: string,
  ): Promise<void> {
    const scoped = scopedNamespace(desc, namespace);
    await request(connId, {
      method: "post",
      path: collectionPath(desc, scoped),
      body: manifest,
      contentType: "application/yaml",
      acknowledged: true,
    });
    refresh(connId, desc.key, scoped);
  }

  /**
   * Whether a terminal can be opened inside this pod.
   *
   * Only a running pod has a container to attach to. A completed one still has a `Running` phase in
   * some controllers' eyes after its containers exited, so the check reads the status column rather
   * than the phase.
   */
  function isRunning(obj: K8sObject): boolean {
    return podStatus(obj) === "Running";
  }

  return {
    connections,
    caches,
    metrics,
    overviews,
    overviewFailures,
    liveUpdates,
    connect,
    reconnect,
    disconnectSession,
    connectionIdOf,
    connectionOf,
    release,
    subscribe,
    unsubscribe,
    refresh,
    relist,
    rowsOf,
    isLoading,
    failureOf,
    watchFailureOf,
    request,
    metricsOf,
    loadMetrics,
    usageOfPod,
    usageOfNode,
    clusterUsage,
    loadOverview,
    overviewAge,
    connectionIdOfObject,
    replaceObject,
    deleteObject,
    restartWorkload,
    scaleWorkload,
    setUnschedulable,
    drainNode,
    openExec,
    createObject,
    isRunning,
  };
});
