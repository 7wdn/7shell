/**
 * The container store: one engine per tab, and the streams that outlive a keystroke.
 *
 * What can only be seen here is the wiring the views depend on — the connection id written back onto
 * the tab so `terminals` can close it, an answer that arrives after its tab went away being thrown
 * away rather than applied, one stream subscription for every stream, and a stop that is real (the
 * line after it reaches nobody). The runtimes' own parsing is tested against their output in the
 * Rust crate; the folding of their progress lines into a bar is tested here, because that is where
 * it is written.
 */
import { createPinia, setActivePinia } from "pinia";
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  containerConnect: vi.fn(),
  containerDisconnect: vi.fn(async () => {}),
  containerList: vi.fn(async () => []),
  containerImages: vi.fn(async () => []),
  containerRuntimeInfo: vi.fn(),
  containerNamespaces: vi.fn(async () => []),
  containerSetNamespace: vi.fn(async () => {}),
  containerAction: vi.fn(async () => {}),
  containerRename: vi.fn(async () => {}),
  containerCreate: vi.fn(async () => {}),
  containerInspect: vi.fn(),
  containerLogsStart: vi.fn(),
  containerStreamStop: vi.fn(async () => {}),
  containerExecOpen: vi.fn(),
  containerImageInspect: vi.fn(),
  containerImageHistory: vi.fn(async () => []),
  containerImagePull: vi.fn(),
  containerImagePush: vi.fn(),
  containerImageRemove: vi.fn(async () => {}),
  containerImagePrune: vi.fn(async () => {}),
  containerImageTag: vi.fn(async () => {}),
  containerImageCommit: vi.fn(),
  containerRegistryLogin: vi.fn(async () => {}),
  onContainerStreamLine: vi.fn(),
  onContainerStreamEnd: vi.fn(),
}));

import {
  containerAction,
  containerConnect,
  containerDisconnect,
  containerImageCommit,
  containerImagePull,
  containerImages,
  containerList,
  containerLogsStart,
  containerRuntimeInfo,
  containerSetNamespace,
  containerStreamStop,
  onContainerStreamEnd,
  onContainerStreamLine,
} from "@/lib/backend/tauri";
import {
  appendLogLine,
  applyImageProgress,
  CONTAINER_ACTIONS,
  filterContainers,
  filterImages,
  imageProgressPercent,
  imageRef,
  layerCounts,
  LOG_LINE_LIMIT,
  matchesImage,
  parseByteSize,
  relatedContainers,
  shortId,
  splitLogLine,
  supportsCommit,
  supportsInsecure,
  supportsNamespace,
  supportsRename,
  supportsRestart,
  supportsTag,
  supportsTerminalOptions,
  useContainerStore,
  type LayerProgress,
} from "@/stores/container";
import type { ContainerTab } from "@/stores/terminals";
import type {
  ContainerImage,
  ContainerRuntimeInfo,
  ContainerStreamEnd,
  ContainerStreamLine,
  ContainerSummary,
} from "@/types/container";

const connectMock = vi.mocked(containerConnect);
const disconnectMock = vi.mocked(containerDisconnect);
const listMock = vi.mocked(containerList);
const imagesMock = vi.mocked(containerImages);
const infoMock = vi.mocked(containerRuntimeInfo);
const namespaceSetMock = vi.mocked(containerSetNamespace);
const actionMock = vi.mocked(containerAction);
const logsStartMock = vi.mocked(containerLogsStart);
const streamStopMock = vi.mocked(containerStreamStop);
const pullMock = vi.mocked(containerImagePull);
const commitMock = vi.mocked(containerImageCommit);
const lineMock = vi.mocked(onContainerStreamLine);
const endMock = vi.mocked(onContainerStreamEnd);

function container(overrides: Partial<ContainerSummary> = {}): ContainerSummary {
  return {
    id: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    name: "web",
    image: "nginx:latest",
    state: "running",
    status: "Up 2 hours",
    ports: "0.0.0.0:8080->80/tcp",
    createdAt: "2026-11-10 10:00:00 +0000 UTC",
    ...overrides,
  };
}

function image(overrides: Partial<ContainerImage> = {}): ContainerImage {
  return {
    id: "sha256:abcdef0123456789abcdef0123456789",
    repository: "nginx",
    tag: "latest",
    size: "187MB",
    createdAt: "2026-11-10 10:00:00 +0000 UTC",
    ...overrides,
  };
}

function runtimeInfo(): ContainerRuntimeInfo {
  return {
    runtime: "docker",
    clientVersion: "28.0.0",
    serverVersion: "28.0.0",
    cpus: 8,
    memory: 16 * 1024 ** 3,
    containersTotal: 1,
    containersByState: [{ state: "running", count: 1 }],
    imagesTotal: 1,
    imagesTagged: 1,
    imagesDangling: 0,
  };
}

function tab(runtime: "docker" | "podman" | "nerdctl" | "wslc" = "docker"): ContainerTab {
  return {
    kind: "container",
    id: "tab-1",
    session: {
      id: "session-1",
      name: "docker on lab",
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

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void; reject: (reason: unknown) => void } {
  let resolve: (value: T) => void = () => {};
  let reject: (reason: unknown) => void = () => {};
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

let lineListener: ((payload: ContainerStreamLine) => void) | null = null;
let endListener: ((payload: ContainerStreamEnd) => void) | null = null;
let stopLine: () => void;
let stopEnd: () => void;

beforeEach(() => {
  setActivePinia(createPinia());
  vi.clearAllMocks();

  connectMock.mockResolvedValue("conn-1");
  listMock.mockResolvedValue([container()]);
  imagesMock.mockResolvedValue([image()]);
  infoMock.mockResolvedValue(runtimeInfo());
  logsStartMock.mockResolvedValue("log-1");
  pullMock.mockResolvedValue("pull-1");
  streamStopMock.mockResolvedValue(undefined);

  lineListener = null;
  endListener = null;
  stopLine = vi.fn();
  stopEnd = vi.fn();
  lineMock.mockImplementation(async (listener) => {
    lineListener = listener;
    return stopLine;
  });
  endMock.mockImplementation(async (listener) => {
    endListener = listener;
    return stopEnd;
  });
});

afterEach(() => {
  useContainerStore().forget("tab-1");
});

function emitLine(streamId: string, line: string): void {
  if (!lineListener) {
    throw new Error("the store never subscribed to stream lines");
  }
  lineListener({ streamId, line });
}

function emitEnd(streamId: string, error?: string): void {
  if (!endListener) {
    throw new Error("the store never subscribed to stream ends");
  }
  endListener({ streamId, error });
}

describe("container store connection", () => {
  it("opens one connection per tab, writes it onto the tab, and reads the three lists", async () => {
    const engine = useContainerStore();
    const opened = tab();

    await engine.connect(opened);

    expect(connectMock).toHaveBeenCalledWith("session-1");
    expect(opened.connectionId).toBe("conn-1");
    expect(engine.sessionFor("tab-1")?.connectionId).toBe("conn-1");
    expect(engine.sessionFor("tab-1")?.containers).toHaveLength(1);
    expect(engine.sessionFor("tab-1")?.images).toHaveLength(1);
    expect(engine.sessionFor("tab-1")?.info?.serverVersion).toBe("28.0.0");
    expect(engine.sessionFor("tab-1")?.loading).toBe(false);
  });

  it("does not open a second connection when the pane mounts twice", async () => {
    const engine = useContainerStore();
    const opened = tab();

    await engine.connect(opened);
    await engine.connect(opened);

    expect(connectMock).toHaveBeenCalledTimes(1);
  });

  it("lets go of a connection that answered after its tab had gone", async () => {
    const engine = useContainerStore();
    const opened = tab();
    const answer = deferred<string>();
    connectMock.mockReturnValue(answer.promise);

    const opening = engine.connect(opened);
    engine.forget("tab-1");
    answer.resolve("conn-late");
    await opening;

    expect(disconnectMock).toHaveBeenCalledWith("conn-late");
    expect(engine.sessionFor("tab-1")).toBeUndefined();
    expect(opened.connectionId).toBeNull();
  });

  it("keeps a refusal on the session and lets a retry try again", async () => {
    const engine = useContainerStore();
    const opened = tab();
    connectMock.mockRejectedValueOnce(new Error("npipe is not there"));

    await engine.connect(opened);

    expect(engine.sessionFor("tab-1")?.connectionId).toBeNull();
    expect(engine.sessionFor("tab-1")?.error?.code).toBe("APP-INTERNAL");

    await engine.connect(opened);
    expect(engine.sessionFor("tab-1")?.connectionId).toBe("conn-1");
  });

  it("disconnects when it is closed, and forgets without disconnecting when the pane unmounts", async () => {
    const engine = useContainerStore();
    const opened = tab();

    await engine.connect(opened);
    engine.forget("tab-1");
    expect(disconnectMock).not.toHaveBeenCalled();

    await engine.connect(opened);
    await engine.close("tab-1");
    expect(disconnectMock).toHaveBeenCalledWith("conn-1");
    expect(engine.sessionFor("tab-1")).toBeUndefined();
  });
});

describe("container store streams", () => {
  it("subscribes to the stream events once, however many streams are started", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());

    logsStartMock.mockResolvedValueOnce("log-1").mockResolvedValueOnce("log-2");
    await engine.startLogs("tab-1", "c1", 500, false, { onLine: () => {}, onEnd: () => {} });
    await engine.startLogs("tab-1", "c2", 500, false, { onLine: () => {}, onEnd: () => {} });

    expect(lineMock).toHaveBeenCalledTimes(1);
    expect(endMock).toHaveBeenCalledTimes(1);
  });

  it("routes lines by stream id and stops routing them once the stream is stopped", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());

    const lines: string[] = [];
    const ended: (string | undefined)[] = [];
    await engine.startLogs("tab-1", "c1", 500, false, {
      onLine: (line) => lines.push(line),
      onEnd: (error) => ended.push(error),
    });

    emitLine("log-1", "first");
    expect(lines).toEqual(["first"]);

    await engine.stopStream("log-1");
    expect(streamStopMock).toHaveBeenCalledWith("log-1");
    emitLine("log-1", "after the stop");
    expect(lines).toEqual(["first"]);

    emitEnd("log-1", "ignored");
    expect(ended).toEqual([]);
  });

  it("hands the end of a stream to its own view, and only to it", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());

    const first: (string | undefined)[] = [];
    const second: (string | undefined)[] = [];
    logsStartMock.mockResolvedValueOnce("log-1").mockResolvedValueOnce("log-2");
    await engine.startLogs("tab-1", "c1", 500, false, { onLine: () => {}, onEnd: (error) => first.push(error) });
    await engine.startLogs("tab-1", "c2", 500, false, { onLine: () => {}, onEnd: (error) => second.push(error) });

    emitEnd("log-1", "the container went away");
    expect(first).toEqual(["the container went away"]);
    expect(second).toEqual([]);
  });

  it("stops every stream a session owns when the pane forgets it, and lets the listener go", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());
    logsStartMock.mockResolvedValueOnce("log-1").mockResolvedValueOnce("log-2");
    await engine.startLogs("tab-1", "c1", 500, false, { onLine: () => {}, onEnd: () => {} });
    await engine.startLogs("tab-1", "c2", 500, false, { onLine: () => {}, onEnd: () => {} });

    engine.forget("tab-1");
    await Promise.resolve();

    expect(streamStopMock.mock.calls.map((call) => call[0]).sort()).toEqual(["log-1", "log-2"]);
    expect(stopLine).toHaveBeenCalled();
    expect(stopEnd).toHaveBeenCalled();
  });
});

describe("container store actions", () => {
  it("passes the acknowledgement of a destructive action through and reads the list again", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());
    listMock.mockClear();

    await engine.action("tab-1", "c1", CONTAINER_ACTIONS.stop, true);

    expect(actionMock).toHaveBeenCalledWith("conn-1", "c1", "stop", true);
    expect(listMock).toHaveBeenCalledWith("conn-1");
  });

  it("keeps a refusal from an action on the session and rethrows it", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());
    actionMock.mockRejectedValueOnce(new Error("no such container"));

    await expect(engine.action("tab-1", "c1", CONTAINER_ACTIONS.remove, true)).rejects.toMatchObject({
      code: "APP-INTERNAL",
    });
    expect(engine.sessionFor("tab-1")?.error?.code).toBe("APP-INTERNAL");
  });

  it("switches namespace in the engine and reads both lists again", async () => {
    const engine = useContainerStore();
    await engine.connect(tab("nerdctl"));
    listMock.mockClear();
    imagesMock.mockClear();

    await engine.setNamespace("tab-1", "k8s.io");

    expect(namespaceSetMock).toHaveBeenCalledWith("conn-1", "k8s.io");
    expect(engine.sessionFor("tab-1")?.namespace).toBe("k8s.io");
    expect(listMock).toHaveBeenCalledTimes(1);
    expect(imagesMock).toHaveBeenCalledTimes(1);
  });

  it("sends a commit with the acknowledgement the dialog collected", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());
    commitMock.mockResolvedValue("sha256:beef");

    await engine.commit("tab-1", { containerId: "c1", image: "web:snapshot" });

    expect(commitMock).toHaveBeenCalledWith("conn-1", { containerId: "c1", image: "web:snapshot" }, true);
  });
});

describe("image transfers", () => {
  it("keeps one row per transfer, folds the runtime's lines into progress, and reloads images when a pull ends", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());
    imagesMock.mockClear();

    await engine.pullImage("tab-1", { image: "redis:7" });
    const task = engine.sessionFor("tab-1")?.tasks[0];
    expect(task?.status).toBe("running");
    expect(task?.streamId).toBe("pull-1");

    emitLine("pull-1", "3f4e: Pulling fs layer");
    emitLine("pull-1", "3f4e: Downloading [====>   ]  5.25MB/52.5MB");
    emitLine("pull-1", "9ab0: Pulling fs layer");
    emitLine("pull-1", "9ab0: Pull complete");

    expect(task?.lines).toHaveLength(4);
    expect(layerCounts(task?.layers ?? new Map<string, LayerProgress>())).toEqual({ done: 1, total: 2 });

    emitLine("pull-1", "3f4e: Pull complete");
    expect(imageProgressPercent(task?.layers ?? new Map<string, LayerProgress>())).toBe(100);

    emitEnd("pull-1");
    expect(task?.status).toBe("done");
    expect(task?.streamId).toBeNull();
    expect(imagesMock).toHaveBeenCalledTimes(1);
  });

  it("marks a transfer stopped without letting its late end event overwrite the answer", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());

    await engine.pullImage("tab-1", { image: "redis:7" });
    const task = engine.sessionFor("tab-1")?.tasks[0];
    if (!task) {
      throw new Error("the transfer never appeared");
    }

    await engine.stopTask("tab-1", task.id);
    expect(task.status).toBe("stopped");
    expect(streamStopMock).toHaveBeenCalledWith("pull-1");

    emitEnd("pull-1", "killed");
    expect(task.status).toBe("stopped");
    expect(task.streamError).toBeNull();
  });

  it("keeps a refusal to start on the row rather than throwing at the panel", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());
    pullMock.mockRejectedValueOnce(new Error("registry unreachable"));

    await engine.pullImage("tab-1", { image: "redis:7" });

    const task = engine.sessionFor("tab-1")?.tasks[0];
    expect(task?.status).toBe("failed");
    expect(task?.failure?.code).toBe("APP-INTERNAL");
    expect(task?.endedAt).toBeGreaterThan(0);
  });

  it("removes a finished row but leaves a running one alone", async () => {
    const engine = useContainerStore();
    await engine.connect(tab());
    await engine.pullImage("tab-1", { image: "redis:7" });
    const task = engine.sessionFor("tab-1")?.tasks[0];
    if (!task) {
      throw new Error("the transfer never appeared");
    }

    engine.clearTask("tab-1", task.id);
    expect(engine.sessionFor("tab-1")?.tasks).toHaveLength(1);

    emitEnd("pull-1");
    engine.clearTask("tab-1", task.id);
    expect(engine.sessionFor("tab-1")?.tasks).toHaveLength(0);
  });
});

describe("runtime capabilities", () => {
  it("draws only the actions a runtime can carry out", () => {
    expect(supportsRestart("wslc")).toBe(false);
    expect(supportsRestart("podman")).toBe(true);
    expect(supportsRename("wslc")).toBe(false);
    expect(supportsCommit("docker")).toBe(true);
    expect(supportsCommit("podman")).toBe(false);
    expect(supportsTag("wslc")).toBe(false);
    expect(supportsInsecure("docker")).toBe(false);
    expect(supportsInsecure("nerdctl")).toBe(true);
    expect(supportsNamespace("nerdctl")).toBe(true);
    expect(supportsNamespace("docker")).toBe(false);
    expect(supportsTerminalOptions("wslc")).toBe(false);
  });
});

describe("pure helpers", () => {
  it("shortens an id the way the runtime prints it, without the digest prefix", () => {
    expect(shortId("sha256:abcdef0123456789")).toBe("abcdef012345");
    expect(shortId("sha256:abc")).toBe("abc");
    expect(shortId("abcdef0123456789")).toBe("abcdef012345");
  });

  it("names an image by its tag, and by its id when it has no tag left", () => {
    expect(imageRef(image())).toBe("nginx:latest");
    expect(imageRef(image({ tag: "<none>" }))).toBe("sha256:abcdef0123456789abcdef0123456789");
    expect(imageRef(image({ repository: "<none>" }))).toBe("sha256:abcdef0123456789abcdef0123456789");
  });

  it("recognises a container by the tag it was started from and by an image id", () => {
    expect(matchesImage(container(), "nginx:latest")).toBe(true);
    expect(matchesImage(container({ image: "abcdef0123456789" }), "sha256:abcdef012345")).toBe(true);
    expect(matchesImage(container(), "redis:7")).toBe(false);
    expect(relatedContainers([container(), container({ id: "b", image: "redis:7" })], image())).toBe(1);
  });

  it("filters containers by running state, state, image and text together", () => {
    const containers = [
      container(),
      container({ id: "b", name: "worker", state: "exited", image: "redis:7" }),
      container({ id: "c", name: "sidecar", state: "paused" }),
    ];

    expect(filterContainers(containers, { runningOnly: true })).toHaveLength(1);
    expect(filterContainers(containers, { runningOnly: true, state: "exited" })).toHaveLength(0);
    expect(filterContainers(containers, { state: "paused" }).map((entry) => entry.name)).toEqual(["sidecar"]);
    expect(filterContainers(containers, { image: "redis:7" }).map((entry) => entry.name)).toEqual(["worker"]);
    expect(filterContainers(containers, { text: "SIDECAR" }).map((entry) => entry.name)).toEqual(["sidecar"]);
    expect(filterContainers(containers, { text: "0123456789abcdef" })).toHaveLength(1);
  });

  it("filters images by repository, tag or id and leaves them in order", () => {
    const images = [image(), image({ id: "sha256:beef", repository: "redis", tag: "7" })];
    expect(filterImages(images, "")).toHaveLength(2);
    expect(filterImages(images, "redis")).toHaveLength(1);
    expect(filterImages(images, "beef")).toHaveLength(1);
    expect(filterImages(images, "nothing")).toHaveLength(0);
  });

  it("reads the byte counts out of a runtime line", () => {
    expect(parseByteSize("5", "B")).toBe(5);
    expect(parseByteSize("5.25", "MB")).toBe(Math.round(5.25 * 1024 ** 2));
    expect(parseByteSize("not a number", "MB")).toBe(0);
  });

  it("folds both the per-layer and the older aggregate output into one bar", () => {
    const layers = new Map<string, LayerProgress>();
    expect(imageProgressPercent(layers)).toBeNull();

    applyImageProgress(layers, "3f4e: Pulling fs layer");
    applyImageProgress(layers, "3f4e: Downloading [==>   ]  2MB/4MB");
    expect(imageProgressPercent(layers)).toBe(50);

    applyImageProgress(layers, "this is not a layer line");
    expect(imageProgressPercent(layers)).toBe(50);

    applyImageProgress(layers, "3f4e: Pull complete");
    expect(imageProgressPercent(layers)).toBe(100);

    const aggregate = new Map<string, LayerProgress>();
    applyImageProgress(aggregate, "Pulling fs layer");
    applyImageProgress(aggregate, "Downloading [=>    ]  1MB/2MB");
    applyImageProgress(aggregate, "Download complete");
    expect(imageProgressPercent(aggregate)).toBe(100);
  });

  it("splits a log line's timestamp from its message only when there is one", () => {
    expect(splitLogLine("2026-11-10T10:00:00.123456789Z hello there")).toEqual({
      timestamp: "2026-11-10T10:00:00.123456789Z",
      message: "hello there",
    });
    expect(splitLogLine("hello there")).toEqual({ timestamp: "", message: "hello there" });
  });

  it("keeps a log buffer bounded, dropping the oldest lines", () => {
    const buffer: string[] = [];
    for (let index = 0; index < LOG_LINE_LIMIT + 5; index += 1) {
      appendLogLine(buffer, `line ${index}`);
    }
    expect(buffer).toHaveLength(LOG_LINE_LIMIT);
    expect(buffer[0]).toBe("line 5");
  });
});
