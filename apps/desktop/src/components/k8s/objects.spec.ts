/**
 * Reading objects: the readings a pod table lives on.
 *
 * A pod's phase is not its status — the case people open a cluster to find is a `Running` pod with a
 * container stuck in `ImagePullBackOff` — and the counts beside it have to come from the statuses
 * rather than from the replica counters, which are what a rollout is in the middle of changing.
 */
import { describe, expect, it } from "vitest";

import {
  accessModesOf,
  age,
  allocatableOf,
  asObject,
  capacityOf,
  completionsText,
  containerNamesOf,
  idOf,
  imagesText,
  mapText,
  nameOf,
  namespaceOf,
  nodeReady,
  nodeRoles,
  podReady,
  podRequests,
  podRestarts,
  podStatus,
  podTone,
  replicaSummary,
  replicaTone,
  servicePorts,
  textAt,
  type K8sObject,
} from "@/components/k8s/objects";

function pod(overrides: Partial<K8sObject> = {}): K8sObject {
  return {
    metadata: { name: "web-0", namespace: "default", uid: "uid-0" },
    spec: { containers: [{ name: "main", image: "nginx:1.27" }] },
    status: { phase: "Running", containerStatuses: [{ ready: true, restartCount: 1 }] },
    ...overrides,
  };
}

describe("identity", () => {
  it("tells two objects apart by uid, falling back to the qualified name", () => {
    expect(idOf({ metadata: { uid: "uid-1", name: "web" } })).toBe("uid-1");
    expect(idOf({ metadata: { name: "web", namespace: "default" } })).toBe("default/web");
  });

  it("answers empty strings for fields the server has not filled in yet", () => {
    expect(nameOf({})).toBe("");
    expect(namespaceOf({ metadata: {} })).toBe("");
    expect(textAt({ metadata: { labels: { app: "web" } } }, "metadata", "labels", "app")).toBe("web");
    expect(asObject([])).toBeNull();
  });
});

describe("pod status", () => {
  it("prefers a container's own reason over the phase", () => {
    const stuck = pod({
      status: {
        phase: "Running",
        containerStatuses: [{ ready: false, state: { waiting: { reason: "ImagePullBackOff" } } }],
      },
    });
    expect(podStatus(stuck)).toBe("ImagePullBackOff");
  });

  it("calls a pod with a deletion timestamp terminating whatever its containers say", () => {
    expect(podStatus(pod({ metadata: { name: "web-0", deletionTimestamp: "2026-11-10T00:00:00Z" } }))).toBe(
      "Terminating",
    );
  });

  it("counts readiness and restarts across the containers", () => {
    const two = pod({
      status: {
        phase: "Running",
        containerStatuses: [
          { ready: true, restartCount: 2 },
          { ready: false, restartCount: 3 },
        ],
      },
    });
    expect(podReady(two)).toBe("1/2");
    expect(podRestarts(two)).toBe(5);
    expect(podTone(two)).toBe("warn");
  });

  it("marks a failed pod as an error and a pending one as worth a look", () => {
    expect(podTone(pod({ status: { phase: "Failed" } }))).toBe("err");
    expect(podTone(pod({ status: { phase: "Pending" } }))).toBe("warn");
    expect(podTone(pod())).toBe("");
  });
});

describe("pod readings", () => {
  it("sums the requests of every container, in the units the percentage columns divide", () => {
    const two = pod({
      spec: {
        containers: [
          { name: "main", resources: { requests: { cpu: "100m", memory: "128Mi" } } },
          { name: "sidecar", resources: { requests: { cpu: "0.5", memory: "1Gi" } } },
        ],
      },
    });
    expect(podRequests(two)).toEqual({ cpu: 600, mem: 128 * 1024 ** 2 + 1024 ** 3 });
  });

  it("lists container names and images in manifest order", () => {
    const two = pod({
      spec: {
        containers: [
          { name: "main", image: "nginx:1.27" },
          { name: "sidecar", image: "busybox:1.36" },
        ],
      },
    });
    expect(containerNamesOf(two)).toEqual(["main", "sidecar"]);
    expect(imagesText(two)).toBe("nginx:1.27\nbusybox:1.36");
  });
});

describe("workloads", () => {
  it("reads replicas from the spec and readiness from the status", () => {
    const deployment = {
      spec: { replicas: 3 },
      status: { readyReplicas: 2 },
    };
    expect(replicaSummary(deployment)).toBe("2/3");
    expect(replicaTone(deployment)).toBe("warn");
    expect(replicaTone({ spec: { replicas: 3 }, status: { readyReplicas: 0 } })).toBe("err");
    expect(replicaTone({ spec: { replicas: 3 }, status: { readyReplicas: 3 } })).toBe("");
  });

  it("counts a DaemonSet by what it scheduled, because it has no replica count", () => {
    expect(replicaSummary({ status: { desiredNumberScheduled: 4, numberReady: 3 } })).toBe("3/4");
  });

  it("reads a job's completions as done over wanted", () => {
    expect(completionsText({ spec: { completions: 3 }, status: { succeeded: 1 } })).toBe("1/3");
    expect(completionsText({})).toBe("0/1");
  });
});

describe("services, nodes and storage", () => {
  it("writes a service's ports the way a person reads them", () => {
    const service = {
      spec: {
        ports: [
          { name: "http", port: 80, nodePort: 30080, protocol: "TCP" },
          { port: 53, protocol: "UDP" },
        ],
      },
    };
    expect(servicePorts(service)).toBe("http:80→30080 53/UDP");
  });

  it("reads readiness, roles and allocatable from a node", () => {
    const node = {
      metadata: { labels: { "node-role.kubernetes.io/control-plane": "", "kubernetes.io/os": "linux" } },
      status: {
        conditions: [{ type: "Ready", status: "True" }],
        allocatable: { cpu: "8", memory: "16Gi" },
      },
    };
    expect(nodeReady(node)).toBe(true);
    expect(nodeRoles(node)).toBe("control-plane");
    expect(allocatableOf(node)).toEqual({ cpu: 8000, mem: 16 * 1024 ** 3 });
    expect(nodeReady({ status: { conditions: [{ type: "Ready", status: "False" }] } })).toBe(false);
  });

  it("reads capacity and access modes from wherever the kind keeps them", () => {
    expect(capacityOf({ status: { capacity: { storage: "10Gi" } } })).toBe("10Gi");
    expect(capacityOf({ spec: { resources: { requests: { storage: "5Gi" } } } })).toBe("5Gi");
    expect(accessModesOf({ spec: { accessModes: ["ReadWriteOnce", "ReadOnlyMany"] } })).toBe(
      "ReadWriteOnce, ReadOnlyMany",
    );
  });

  it("spells a label map as key=value lines, or a dash when there is none", () => {
    expect(mapText({ app: "web", tier: "backend" })).toBe("app=web\ntier=backend");
    expect(mapText(undefined)).toBe("—");
  });
});

describe("age", () => {
  it("answers in the largest unit that fits, and a dash for a timestamp it cannot read", () => {
    const now = Date.parse("2026-11-10T12:00:00Z");
    expect(age("2026-11-10T11:59:30Z", now)).toBe("30s");
    expect(age("2026-11-10T11:50:00Z", now)).toBe("10m");
    expect(age("2026-11-10T09:00:00Z", now)).toBe("3h");
    expect(age("2026-11-08T12:00:00Z", now)).toBe("2d");
    expect(age("", now)).toBe("—");
    expect(age("yesterday", now)).toBe("—");
  });
});
