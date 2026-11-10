/**
 * One container engine per open tab: what it holds, what is being read from it, and the streams
 * that are still arriving.
 *
 * Four rules shape this file, and each of them is here because the alternative leaves something
 * behind:
 *
 * 1. **A tab owns its connection, by id.** `terminals` closes the tab and lets the connection go by
 *    way of `tab.connectionId`; the store writes that field the moment the engine answers, so a
 *    connection can never outlive the only tab that could show it. The pane itself only *forgets*
 *    its state on unmount — closing there as well would be a second disconnect of the same id.
 * 2. **Answers are applied only to the session that asked.** A tab closed, or reconnected, while a
 *    list was on its way must not have the old answer land in the new state; every asynchronous
 *    read carries the session it was started for and compares identity before writing.
 * 3. **Streams are looked up, never owned twice.** Logs and image transfers all arrive on one pair of
 *    backend events keyed by stream id. This store subscribes once, routes each line to the sink
 *    registered for that id, and removes the sink before the backend is even asked to stop — so a
 *    late line reaches nothing rather than a view that has been closed.
 * 4. **The runtime is a capability, not a variant.** What `wslc` cannot do (restart, rename, commit,
 *    tag, a per-command TLS switch) is a predicate here, so a button that would fail is not drawn
 *    and the same predicate is what the tests assert.
 */

import { defineStore } from "pinia";
import { ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import type {
  ContainerCommitRequest,
  ContainerCreateRequest,
  ContainerDetail,
  ContainerImage,
  ContainerImageTransfer,
  ContainerRegistryLogin,
  ContainerRuntime,
  ContainerRuntimeInfo,
  ContainerSummary,
  ImageLayer,
} from "@/types/container";
import type { ContainerTab } from "@/stores/terminals";

/** What the container commands accept for a single-container action, in the CLI's own words. */
export const CONTAINER_ACTIONS = {
  start: "start",
  stop: "stop",
  restart: "restart",
  remove: "rm",
} as const;

export type ContainerActionName = (typeof CONTAINER_ACTIONS)[keyof typeof CONTAINER_ACTIONS];

/** How many lines one image transfer keeps for its expandable log. */
export const TASK_LOG_LIMIT = 500;
/** How many lines a log view keeps; older ones fall off the top. */
export const LOG_LINE_LIMIT = 5000;

/** One runtime line of a pull or push, reduced to what a progress bar can be drawn from. */
export interface LayerProgress {
  /** Bytes moved for this layer, as far as the runtime has said. */
  moved: number;
  /** Bytes in the layer, `0` until the runtime has said. */
  total: number;
  done: boolean;
}

export type ImageTaskStatus = "running" | "done" | "failed" | "stopped";

/** One pull or push, kept until somebody clears it. */
export interface ImageTask {
  readonly id: string;
  readonly kind: "pull" | "push";
  readonly image: string;
  status: ImageTaskStatus;
  /** The raw output, capped; what the expandable log draws. */
  lines: string[];
  layers: Map<string, LayerProgress>;
  startedAt: number;
  /** When it stopped being running, for the elapsed reading; `0` while it runs. */
  endedAt: number;
  streamId: string | null;
  /** The refusal, when starting it was refused rather than stopping part-way. */
  failure: ShellError | null;
  /** Why the stream ended, when it ended badly; the backend's own sentence. */
  streamError: string | null;
}

/** Everything one tab knows about its engine. */
export interface ContainerSession {
  connectionId: string | null;
  readonly runtime: ContainerRuntime;
  namespace: string;
  containers: ContainerSummary[];
  images: ContainerImage[];
  namespaces: string[];
  /** The connection is being opened; the lists load afterwards. */
  loading: boolean;
  refreshing: boolean;
  imagesLoading: boolean;
  /** The last failure, kept until a read or an action succeeds. */
  error: ShellError | null;
  info: ContainerRuntimeInfo | null;
  tasks: ImageTask[];
}

/** Where a stream's lines go. Registered before the backend starts writing, removed on stop. */
interface StreamSink {
  readonly session: ContainerSession;
  onLine: (line: string) => void;
  onEnd: (error?: string) => void;
}

/** Per-session counters, so an answer that is no longer wanted cannot overwrite a newer one. */
interface SessionSequences {
  containers: number;
  images: number;
  info: number;
  namespaces: number;
}

/** What a container list is narrowed to. */
export interface ContainerQuery {
  readonly text?: string;
  readonly state?: string | null;
  /** An image reference, as the images view names it. */
  readonly image?: string | null;
  readonly runningOnly?: boolean;
}

// -- pure helpers ------------------------------------------------------------

/**
 * The twelve characters a runtime prints beside a full id.
 *
 * The `sha256:` prefix is dropped first: it is the same for every id, and keeping it would spend
 * six of the twelve characters on it.
 */
export function shortId(id: string): string {
  const raw = id.replace(/^sha256:/, "");
  return raw.length > 12 ? raw.slice(0, 12) : raw;
}

/** How an image is named on a command line or in a container's image column. */
export function imageRef(image: ContainerImage): string {
  if (image.repository === "<none>" || !image.tag || image.tag === "<none>") {
    return image.id;
  }
  return `${image.repository}:${image.tag}`;
}

/**
 * Whether a container was made from an image.
 *
 * Two shapes, because a runtime reports two: the `repo:tag` it was started from, and — for an image
 * with no tag, or one whose tag was removed afterwards — the image id.
 */
export function matchesImage(container: ContainerSummary, reference: string): boolean {
  if (container.image === reference) {
    return true;
  }
  const id = reference.replace(/^sha256:/, "");
  return id !== "" && container.image.replace(/^sha256:/, "").startsWith(id);
}

/** How many listed containers run an image. */
export function relatedContainers(containers: readonly ContainerSummary[], image: ContainerImage): number {
  const reference = imageRef(image);
  return containers.filter((container) => matchesImage(container, reference)).length;
}

/** The containers a query leaves, in the order the backend gave them. */
export function filterContainers(
  containers: readonly ContainerSummary[],
  query: ContainerQuery = {},
): ContainerSummary[] {
  const text = (query.text ?? "").trim().toLowerCase();
  const state = query.state ?? null;
  const image = query.image ?? null;

  return containers.filter((container) => {
    if (query.runningOnly && container.state !== "running") {
      return false;
    }
    if (state !== null && container.state !== state) {
      return false;
    }
    if (image !== null && !matchesImage(container, image)) {
      return false;
    }
    if (text !== "") {
      const haystack = `${container.name}\n${container.image}\n${container.id}`.toLowerCase();
      if (!haystack.includes(text)) {
        return false;
      }
    }
    return true;
  });
}

/** The images a text filter leaves. */
export function filterImages(images: readonly ContainerImage[], text: string): ContainerImage[] {
  const needle = text.trim().toLowerCase();
  if (needle === "") {
    return [...images];
  }
  return images.filter(
    (image) =>
      image.repository.toLowerCase().includes(needle) ||
      image.tag.toLowerCase().includes(needle) ||
      image.id.toLowerCase().includes(needle),
  );
}

/** Whether a runtime can restart a container. The WSL container service cannot. */
export function supportsRestart(runtime: ContainerRuntime): boolean {
  return runtime !== "wslc";
}

/** Whether a runtime can rename a container. */
export function supportsRename(runtime: ContainerRuntime): boolean {
  return runtime !== "wslc";
}

/** Whether a runtime can turn a container into an image. Only the Docker CLI has `commit`. */
export function supportsCommit(runtime: ContainerRuntime): boolean {
  return runtime === "docker";
}

/** Whether a runtime can add a second tag to an image. */
export function supportsTag(runtime: ContainerRuntime): boolean {
  return runtime !== "wslc";
}

/**
 * Whether a pull, push or login can be asked to skip TLS verification for one command.
 *
 * The Docker CLI has no per-command switch — it is a daemon setting — so offering the checkbox
 * there would be a switch that changes nothing.
 */
export function supportsInsecure(runtime: ContainerRuntime): boolean {
  return runtime === "podman" || runtime === "nerdctl";
}

/** Whether the runtime has namespaces at all. Only containerd behind `nerdctl` does. */
export function supportsNamespace(runtime: ContainerRuntime): boolean {
  return runtime === "nerdctl";
}

/** Whether `create` takes `--interactive` and `--tty`. */
export function supportsTerminalOptions(runtime: ContainerRuntime): boolean {
  return runtime !== "wslc";
}

/** One layer state line, with its layer id when the runtime printed one. */
const LAYER_STATE =
  /^\s*(?:(\S+):\s+)?(Pulling fs layer|Waiting|Downloading|Extracting|Verifying Checksum|Download complete|Pull complete|Pushing|Pushed|Preparing|Layer already exists|Already exists)\b(.*)$/;

/** `5.25MB/52.5MB`, which is how both the byte count and the extraction count are printed. */
const SIZE_PAIR = /(\d+(?:\.\d+)?)\s*([kKmMgGtT]?[bB])\s*\/\s*(\d+(?:\.\d+)?)\s*([kKmMgGtT]?[bB])/;

const SIZE_UNITS: Readonly<Record<string, number>> = {
  b: 1,
  kb: 1024,
  mb: 1024 ** 2,
  gb: 1024 ** 3,
  tb: 1024 ** 4,
};

/** `5.25MB` as bytes. */
export function parseByteSize(value: string, unit: string): number {
  const amount = Number(value);
  if (!Number.isFinite(amount)) {
    return 0;
  }
  return Math.round(amount * (SIZE_UNITS[unit.toLowerCase()] ?? 1));
}

/**
 * Folds one output line of a pull or push into the per-layer progress.
 *
 * The runtimes do not print a percentage: they print a line per layer state, and the byte counts
 * beside some of those states. A layer identified by nothing — the older Docker output — is folded
 * into one aggregate entry so a bar still has something to draw.
 */
export function applyImageProgress(layers: Map<string, LayerProgress>, line: string): void {
  const match = LAYER_STATE.exec(line);
  if (!match) {
    return;
  }

  const key = match[1] ?? "";
  const state = match[2] ?? "";
  const rest = match[3] ?? "";

  let layer = layers.get(key);
  if (!layer) {
    layer = { moved: 0, total: 0, done: false };
    layers.set(key, layer);
  }

  const size = SIZE_PAIR.exec(rest);
  if (size) {
    const moved = parseByteSize(size[1] ?? "0", size[2] ?? "b");
    const total = parseByteSize(size[3] ?? "0", size[4] ?? "b");
    if (total > 0) {
      layer.total = total;
    }
    if (moved > layer.moved) {
      layer.moved = moved;
    }
  }

  if (
    state === "Pull complete" ||
    state === "Pushed" ||
    state === "Layer already exists" ||
    state === "Already exists" ||
    state === "Download complete"
  ) {
    layer.done = true;
    if (layer.total > 0) {
      layer.moved = layer.total;
    }
  }
}

/**
 * How far a transfer has got, or `null` while nothing has been seen that a bar could measure.
 *
 * Byte counts are summed across the layers when the runtime prints them; otherwise the fraction of
 * layers finished is the best answer there is. Both are real readings, so the bar never moves
 * backwards by switching between them mid-run.
 */
export function imageProgressPercent(layers: ReadonlyMap<string, LayerProgress>): number | null {
  if (layers.size === 0) {
    return null;
  }

  let total = 0;
  let moved = 0;
  let done = 0;
  for (const layer of layers.values()) {
    if (layer.done) {
      done += 1;
    }
    if (layer.total > 0) {
      total += layer.total;
      moved += layer.done ? layer.total : Math.min(layer.moved, layer.total);
    }
  }

  if (total > 0) {
    return Math.min(100, Math.round((moved / total) * 100));
  }
  return Math.round((done / layers.size) * 100);
}

/** How many layers the runtime has named, and how many of them finished. */
export function layerCounts(layers: ReadonlyMap<string, LayerProgress>): { done: number; total: number } {
  let done = 0;
  for (const layer of layers.values()) {
    if (layer.done) {
      done += 1;
    }
  }
  return { done, total: layers.size };
}

/** A log line split into the timestamp the runtime prefixed and the message itself. */
export function splitLogLine(line: string): { timestamp: string; message: string } {
  const match = /^(\d{4}-\d{2}-\d{2}T\S+)\s(.*)$/s.exec(line);
  if (!match) {
    return { timestamp: "", message: line };
  }
  return { timestamp: match[1] ?? "", message: match[2] ?? "" };
}

/** Appends a line to a bounded buffer, dropping the oldest once it is full. */
export function appendLogLine(buffer: string[], line: string, limit = LOG_LINE_LIMIT): void {
  buffer.push(line);
  if (buffer.length > limit) {
    buffer.splice(0, buffer.length - limit);
  }
}

// -- the store ---------------------------------------------------------------

export const useContainerStore = defineStore("container", () => {
  /** The live sessions, by tab id. */
  const sessions = ref<Record<string, ContainerSession>>({});

  /** Routing for the two stream events; empty means no stream is being read. */
  const sinks = new Map<string, StreamSink>();
  const sequences = new WeakMap<ContainerSession, SessionSequences>();
  let listening = false;
  let unlisten: (() => void)[] = [];
  let subscription: Promise<void> | null = null;
  let subscriptionToken = 0;
  let transferSequence = 0;

  function sessionFor(tabId: string): ContainerSession | undefined {
    return sessions.value[tabId];
  }

  function sequencesOf(session: ContainerSession): SessionSequences {
    let counts = sequences.get(session);
    if (!counts) {
      counts = { containers: 0, images: 0, info: 0, namespaces: 0 };
      sequences.set(session, counts);
    }
    return counts;
  }

  function runtimeOf(tab: ContainerTab): ContainerRuntime {
    return tab.session.container?.runtime ?? "docker";
  }

  function namespaceOf(tab: ContainerTab): string {
    const chosen = tab.session.container?.namespace?.trim();
    return chosen && chosen !== "" ? chosen : "default";
  }

  /**
   * Reads the two stream events once for the whole store.
   *
   * One subscription rather than one per view: the events are keyed by stream id, and a second
   * listener would deliver every line twice to whichever two views happened to be open.
   */
  function listen(): Promise<void> {
    if (subscription) {
      return subscription;
    }
    listening = true;
    // A token rather than a boolean: a release that happens while the listeners are being made has
    // to be recognisable when they arrive, or a second start would have its listeners overwritten
    // by the first one's.
    const token = ++subscriptionToken;
    subscription = Promise.all([
      backend.containers.onStreamLine((payload) => {
        sinks.get(payload.streamId)?.onLine(payload.line);
      }),
      backend.containers.onStreamEnd((payload) => {
        const sink = sinks.get(payload.streamId);
        if (!sink) {
          return;
        }
        // Removed before the sink hears about it, so a second end event finds nothing to tell.
        sinks.delete(payload.streamId);
        sink.onEnd(payload.error);
      }),
    ])
      .then((stops) => {
        if (!listening || token !== subscriptionToken) {
          for (const stop of stops) {
            stop();
          }
          return;
        }
        unlisten = stops;
      })
      .catch((error: unknown) => {
        if (token === subscriptionToken) {
          listening = false;
          subscription = null;
        }
        throw error;
      });
    return subscription;
  }

  /** Lets the events go once there is no session left that could register a sink. */
  function maybeRelease(): void {
    if (Object.keys(sessions.value).length > 0 || sinks.size > 0) {
      return;
    }
    listening = false;
    subscriptionToken += 1;
    for (const stop of unlisten) {
      stop();
    }
    unlisten = [];
    subscription = null;
  }

  /**
   * Starts a stream and registers where its lines go.
   *
   * The registration happens after the id exists, which is the earliest moment there is: a line that
   * arrives before the start command answered cannot be routed, and dropping it is the only honest
   * thing to do — the alternative is a buffer keyed by a stream nobody has named yet.
   */
  async function openStream(
    tabId: string,
    session: ContainerSession,
    start: () => Promise<string>,
    handlers: { onLine: (line: string) => void; onEnd: (error?: string) => void },
  ): Promise<string | null> {
    await listen();
    const streamId = await start();
    if (sessions.value[tabId] !== session) {
      // The tab went away while the stream was being started: stop it and tell nobody.
      try {
        await backend.containers.streamStop(streamId);
      } catch {
        // The backend has no stream left to stop, which is the outcome that was wanted anyway.
      }
      handlers.onEnd();
      return null;
    }
    sinks.set(streamId, { session, onLine: handlers.onLine, onEnd: handlers.onEnd });
    return streamId;
  }

  /** Stops a stream: the router forgets it first, then the backend is asked to stop writing. */
  async function stopStream(streamId: string): Promise<void> {
    sinks.delete(streamId);
    try {
      await backend.containers.streamStop(streamId);
    } catch {
      // It is already gone from this side; a backend that forgot it has nothing to report.
    }
  }

  /** Stops every stream a session owns. Used when the tab goes away. */
  function stopSessionStreams(session: ContainerSession): void {
    // Deleting the entry being visited is what `Map` permits; the remaining entries are visited.
    for (const [streamId, sink] of sinks) {
      if (sink.session !== session) {
        continue;
      }
      sinks.delete(streamId);
      void backend.containers.streamStop(streamId).catch(() => {
        // Same reasoning as stopStream: the stream is already unreachable from here.
      });
    }
  }

  /**
   * Opens the engine for a tab.
   *
   * Safe to call twice — a second call while the first is running does nothing, and a call after a
   * refusal starts over — because the pane mounts and retries through the same entry.
   */
  async function connect(tab: ContainerTab): Promise<void> {
    const existing = sessions.value[tab.id];
    if (existing && (existing.connectionId !== null || existing.loading)) {
      return;
    }

    const fresh: ContainerSession = {
      connectionId: null,
      runtime: runtimeOf(tab),
      namespace: namespaceOf(tab),
      containers: [],
      images: [],
      namespaces: [],
      loading: true,
      refreshing: false,
      imagesLoading: false,
      error: null,
      info: null,
      tasks: [],
    };
    sessions.value[tab.id] = fresh;
    const session = sessions.value[tab.id];
    if (!session) {
      return;
    }

    try {
      const connectionId = await backend.containers.connect(tab.session.id);
      if (sessions.value[tab.id] !== session) {
        // Closed while connecting: no tab can show this engine, so let it go.
        await backend.containers.disconnect(connectionId).catch(() => {
          // A connection that is already gone is the outcome that was wanted.
        });
        return;
      }
      session.connectionId = connectionId;
      session.loading = false;
      tab.connectionId = connectionId;
    } catch (error) {
      if (sessions.value[tab.id] === session) {
        session.error = asShellError(error, "container_connect");
        session.loading = false;
      }
      return;
    }

    await Promise.all([
      refresh(tab.id),
      loadImages(tab.id),
      loadInfo(tab.id),
      supportsNamespace(session.runtime) ? loadNamespaces(tab.id) : Promise.resolve(),
    ]);
  }

  /**
   * Forgets a tab's session without touching the backend.
   *
   * This is what the pane calls on unmount: the tab's own close path has already let the connection
   * go by `tab.connectionId`, and asking twice would be a second disconnect of an id the backend no
   * longer holds. Streams, however, are this store's to stop, because the backend does not end them
   * when a connection is disconnected.
   */
  function forget(tabId: string): void {
    const session = sessions.value[tabId];
    if (!session) {
      return;
    }
    stopSessionStreams(session);
    delete sessions.value[tabId];
    maybeRelease();
  }

  /** Closes a tab's engine: its streams first, then the connection itself. */
  async function close(tabId: string): Promise<void> {
    const session = sessions.value[tabId];
    const connectionId = session?.connectionId ?? null;
    forget(tabId);
    if (connectionId !== null) {
      try {
        await backend.containers.disconnect(connectionId);
      } catch {
        // The connection is gone from the interface either way.
      }
    }
  }

  /** Re-reads the container list. `force` is for a namespace change, which outranks a read in flight. */
  async function refresh(tabId: string, force = false): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    if (session.refreshing && !force) {
      return;
    }

    const connectionId = session.connectionId;
    const counts = sequencesOf(session);
    const mark = ++counts.containers;
    session.refreshing = true;
    try {
      const containers = await backend.containers.list(connectionId);
      if (sessions.value[tabId] !== session || counts.containers !== mark) {
        return;
      }
      session.containers = containers;
      session.error = null;
    } catch (error) {
      if (sessions.value[tabId] === session && counts.containers === mark) {
        session.error = asShellError(error, "container_list");
      }
    } finally {
      if (sessions.value[tabId] === session && counts.containers === mark) {
        session.refreshing = false;
      }
    }
  }

  async function loadImages(tabId: string): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    const counts = sequencesOf(session);
    const mark = ++counts.images;
    session.imagesLoading = true;
    try {
      const images = await backend.containers.images(connectionId);
      if (sessions.value[tabId] !== session || counts.images !== mark) {
        return;
      }
      session.images = images;
    } catch (error) {
      if (sessions.value[tabId] === session && counts.images === mark) {
        session.error = asShellError(error, "container_images");
      }
    } finally {
      if (sessions.value[tabId] === session && counts.images === mark) {
        session.imagesLoading = false;
      }
    }
  }

  /**
   * Reads what the runtime says about itself.
   *
   * A runtime that cannot answer is not a failure of the page: `wslc` has no `info` in the shape the
   * others do, so the card is drawn empty rather than the whole pane turning red.
   */
  async function loadInfo(tabId: string): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    const counts = sequencesOf(session);
    const mark = ++counts.info;
    try {
      const info = await backend.containers.runtimeInfo(connectionId);
      if (sessions.value[tabId] === session && counts.info === mark) {
        session.info = info;
      }
    } catch {
      if (sessions.value[tabId] === session && counts.info === mark) {
        session.info = null;
      }
    }
  }

  async function loadNamespaces(tabId: string): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    const counts = sequencesOf(session);
    const mark = ++counts.namespaces;
    try {
      const namespaces = await backend.containers.namespaces(connectionId);
      if (sessions.value[tabId] === session && counts.namespaces === mark) {
        session.namespaces = namespaces;
      }
    } catch {
      if (sessions.value[tabId] === session && counts.namespaces === mark) {
        session.namespaces = [];
      }
    }
  }

  /** Switches the containerd namespace, then reads both lists again in it. */
  async function setNamespace(tabId: string, namespace: string): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    try {
      await backend.containers.setNamespace(connectionId, namespace);
    } catch (error) {
      session.error = asShellError(error, "container_set_namespace");
      throw error;
    }
    if (sessions.value[tabId] !== session) {
      return;
    }
    session.namespace = namespace;
    await Promise.all([refresh(tabId, true), loadImages(tabId)]);
  }

  /**
   * Runs one container action.
   *
   * The refusal is thrown as well as kept: the view that asked is where the person is looking, so it
   * can say so in a toast without the pane's banner having to be read first.
   */
  async function action(
    tabId: string,
    containerId: string,
    name: ContainerActionName,
    acknowledged = false,
  ): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    try {
      await backend.containers.action(connectionId, containerId, name, acknowledged);
    } catch (error) {
      const failure = asShellError(error, "container_action");
      session.error = failure;
      throw failure;
    }
    if (sessions.value[tabId] !== session) {
      return;
    }
    session.error = null;
    await refresh(tabId);
  }

  async function rename(tabId: string, containerId: string, name: string): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    try {
      await backend.containers.rename(connectionId, containerId, name);
    } catch (error) {
      const failure = asShellError(error, "container_rename");
      session.error = failure;
      throw failure;
    }
    if (sessions.value[tabId] !== session) {
      return;
    }
    await refresh(tabId);
  }

  async function create(tabId: string, request: ContainerCreateRequest): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    try {
      await backend.containers.create(connectionId, request);
    } catch (error) {
      const failure = asShellError(error, "container_create");
      session.error = failure;
      throw failure;
    }
    if (sessions.value[tabId] !== session) {
      return;
    }
    await refresh(tabId);
  }

  /** One container in full, read when its drawer opens rather than with the list. */
  async function inspect(tabId: string, containerId: string): Promise<ContainerDetail | null> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return null;
    }
    return backend.containers.inspect(session.connectionId, containerId);
  }

  async function startLogs(
    tabId: string,
    containerId: string,
    tail: number,
    timestamps: boolean,
    handlers: { onLine: (line: string) => void; onEnd: (error?: string) => void },
  ): Promise<string | null> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return null;
    }
    const connectionId = session.connectionId;
    return openStream(
      tabId,
      session,
      () => backend.containers.logsStart(connectionId, containerId, tail, timestamps),
      handlers,
    );
  }

  /** Opens the pty the backend made for a container; the terminal pane owns it from there. */
  async function exec(tabId: string, containerId: string, shell = "sh"): Promise<string | null> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return null;
    }
    return backend.containers.execOpen(session.connectionId, containerId, shell);
  }

  /** Keeps a failure this store did not make the call for, so the pane's banner can show it. */
  function reportError(tabId: string, error: unknown, operation: string): void {
    const session = sessions.value[tabId];
    if (session) {
      session.error = asShellError(error, operation);
    }
  }

  function dismissError(tabId: string): void {
    const session = sessions.value[tabId];
    if (session) {
      session.error = null;
    }
  }

  // -- images ----------------------------------------------------------------

  async function imageInspect(tabId: string, image: string): Promise<string | null> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return null;
    }
    return backend.containers.imageInspect(session.connectionId, image);
  }

  async function imageHistory(tabId: string, image: string): Promise<ImageLayer[]> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return [];
    }
    return backend.containers.imageHistory(session.connectionId, image);
  }

  async function removeImage(tabId: string, image: string): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    try {
      await backend.containers.imageRemove(connectionId, image, true);
    } catch (error) {
      const failure = asShellError(error, "container_image_remove");
      session.error = failure;
      throw failure;
    }
    await loadImages(tabId);
  }

  async function pruneImages(tabId: string): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    try {
      await backend.containers.imagePrune(connectionId, true);
    } catch (error) {
      const failure = asShellError(error, "container_image_prune");
      session.error = failure;
      throw failure;
    }
    await loadImages(tabId);
  }

  async function tagImage(tabId: string, image: string, tag: string): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    try {
      await backend.containers.imageTag(connectionId, image, tag);
    } catch (error) {
      const failure = asShellError(error, "container_image_tag");
      session.error = failure;
      throw failure;
    }
    await loadImages(tabId);
  }

  /** Writes a container's file system into a new image. Destructive enough to need the answer first. */
  async function commit(tabId: string, request: ContainerCommitRequest): Promise<string | null> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return null;
    }
    const connectionId = session.connectionId;
    let image: string;
    try {
      image = await backend.containers.imageCommit(connectionId, request, true);
    } catch (error) {
      const failure = asShellError(error, "container_image_commit");
      session.error = failure;
      throw failure;
    }
    await Promise.all([loadImages(tabId), refresh(tabId)]);
    return image;
  }

  /** Folds one output line into a task's log and its progress. */
  function sinkFor(task: ImageTask): StreamSink["onLine"] {
    return (line: string) => {
      appendLogLine(task.lines, line, TASK_LOG_LIMIT);
      applyImageProgress(task.layers, line);
    };
  }

  /**
   * Starts one pull or push, as a row in the task panel.
   *
   * Multiple at once is the point: a pull is minutes on a slow registry, and waiting for it before
   * starting the next one is waiting for no reason. Each has its own stream and its own row, and
   * each finishes — or fails — on its own.
   */
  async function startTransfer(tabId: string, kind: "pull" | "push", request: ContainerImageTransfer): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;

    transferSequence += 1;
    session.tasks.push({
      id: `image-${transferSequence}`,
      kind,
      image: request.image,
      status: "running",
      lines: [],
      layers: new Map<string, LayerProgress>(),
      startedAt: Date.now(),
      endedAt: 0,
      streamId: null,
      failure: null,
      streamError: null,
    });
    const task = session.tasks[session.tasks.length - 1];
    if (!task) {
      return;
    }

    try {
      const streamId = await openStream(
        tabId,
        session,
        () =>
          kind === "pull"
            ? backend.containers.imagePull(connectionId, request)
            : backend.containers.imagePush(connectionId, request),
        {
          onLine: sinkFor(task),
          onEnd: (error) => {
            task.streamId = null;
            // Somebody already answered this one by stopping it; the stop stands.
            if (task.status !== "running") {
              return;
            }
            task.endedAt = Date.now();
            if (error) {
              task.status = "failed";
              task.streamError = error;
              return;
            }
            task.status = "done";
            if (kind === "pull") {
              void loadImages(tabId);
            }
          },
        },
      );
      if (streamId !== null) {
        task.streamId = streamId;
      } else if (task.status === "running") {
        // The tab went away before the stream could be registered; there is no row to draw.
        task.status = "failed";
        task.endedAt = Date.now();
      }
    } catch (error) {
      task.status = "failed";
      task.failure = asShellError(error, kind === "pull" ? "container_image_pull" : "container_image_push");
      task.endedAt = Date.now();
    }
  }

  function pullImage(tabId: string, request: ContainerImageTransfer): Promise<void> {
    return startTransfer(tabId, "pull", request);
  }

  function pushImage(tabId: string, request: ContainerImageTransfer): Promise<void> {
    return startTransfer(tabId, "push", request);
  }

  async function stopTask(tabId: string, taskId: string): Promise<void> {
    const session = sessions.value[tabId];
    const task = session?.tasks.find((entry) => entry.id === taskId);
    if (!task || task.status !== "running") {
      return;
    }
    task.status = "stopped";
    task.endedAt = Date.now();
    const streamId = task.streamId;
    task.streamId = null;
    if (streamId !== null) {
      await stopStream(streamId);
    }
  }

  /** Takes a finished row away. A running one stays: it still has bytes to show and a stop button. */
  function clearTask(tabId: string, taskId: string): void {
    const session = sessions.value[tabId];
    if (!session) {
      return;
    }
    session.tasks = session.tasks.filter((task) => task.id !== taskId || task.status === "running");
  }

  function clearFinishedTasks(tabId: string): void {
    const session = sessions.value[tabId];
    if (!session) {
      return;
    }
    session.tasks = session.tasks.filter((task) => task.status === "running");
  }

  async function registryLogin(tabId: string, request: ContainerRegistryLogin): Promise<void> {
    const session = sessions.value[tabId];
    if (!session?.connectionId) {
      return;
    }
    const connectionId = session.connectionId;
    try {
      await backend.containers.registryLogin(connectionId, request);
    } catch (error) {
      const failure = asShellError(error, "container_registry_login");
      session.error = failure;
      throw failure;
    }
  }

  return {
    sessions,
    sessionFor,
    connect,
    forget,
    close,
    refresh,
    loadImages,
    loadInfo,
    loadNamespaces,
    setNamespace,
    action,
    rename,
    create,
    inspect,
    startLogs,
    stopStream,
    exec,
    reportError,
    dismissError,
    imageInspect,
    imageHistory,
    removeImage,
    pruneImages,
    tagImage,
    commit,
    pullImage,
    pushImage,
    stopTask,
    clearTask,
    clearFinishedTasks,
    registryLogin,
  };
});
