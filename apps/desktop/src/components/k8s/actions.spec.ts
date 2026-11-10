/**
 * The writes: request shapes, and which pods a drain may take.
 *
 * These are the rules that would be dangerous to get wrong and invisible in a screenshot — a force
 * delete that quietly sends a different URL, a drain that evicts the coredns pod on the node it is
 * draining, a restart that patches a kind whose controller will not notice.
 */
import { describe, expect, it } from "vitest";

import {
  RESTART_ANNOTATION,
  canRestart,
  createPath,
  deletePath,
  drainPlan,
  evictionBody,
  evictionPath,
  podsOfOwner,
  podsOnNodePath,
  restartPatch,
  restartPath,
  scalePatch,
  scalePath,
  unschedulablePatch,
} from "@/components/k8s/actions";
import { completionsText, nameOf, uidOf } from "@/components/k8s/objects";
import { descriptorOf } from "@/components/k8s/resources";

function descriptor(key: string) {
  const found = descriptorOf(key);
  if (!found) {
    throw new Error(`the ${key} descriptor is missing`);
  }
  return found;
}

describe("delete", () => {
  it("leaves the URL alone unless the delete is forced", () => {
    expect(deletePath("/api/v1/pods/web-0", false)).toBe("/api/v1/pods/web-0");
    expect(deletePath("/api/v1/pods/web-0", true)).toBe(
      "/api/v1/pods/web-0?gracePeriodSeconds=0&propagationPolicy=Background",
    );
  });
});

describe("restart", () => {
  it("stamps the annotation a rollout is driven by", () => {
    const patch = JSON.parse(restartPatch("2026-11-10T09:00:00.000Z")) as Record<string, unknown>;
    expect(patch).toEqual({
      spec: { template: { metadata: { annotations: { [RESTART_ANNOTATION]: "2026-11-10T09:00:00.000Z" } } } },
    });
  });

  it("refuses kinds a restart would not roll", () => {
    expect(canRestart(descriptor("deployments"))).toBe(true);
    expect(canRestart(descriptor("statefulsets"))).toBe(true);
    expect(canRestart(descriptor("daemonsets"))).toBe(true);
    expect(canRestart(descriptor("jobs"))).toBe(false);
    expect(restartPath(descriptor("jobs"), { metadata: { name: "migrate", namespace: "default" } })).toBeNull();
  });

  it("addresses the object itself rather than its collection", () => {
    expect(restartPath(descriptor("deployments"), { metadata: { name: "web", namespace: "default" } })).toBe(
      "/apis/apps/v1/namespaces/default/deployments/web",
    );
  });
});

describe("scale and cordon", () => {
  it("patches the scale subresource with the new replica count", () => {
    expect(scalePath(descriptor("statefulsets"), { metadata: { name: "db", namespace: "data" } })).toBe(
      "/apis/apps/v1/namespaces/data/statefulsets/db/scale",
    );
    expect(scalePatch(4)).toBe(JSON.stringify({ spec: { replicas: 4 } }));
  });

  it("writes the scheduling flag a cordon is", () => {
    expect(unschedulablePatch(true)).toBe(JSON.stringify({ spec: { unschedulable: true } }));
    expect(unschedulablePatch(false)).toBe(JSON.stringify({ spec: { unschedulable: false } }));
  });
});

describe("drain", () => {
  const pod = (name: string, node: string, extra: Record<string, unknown> = {}) => ({
    metadata: { name, namespace: "default", ...extra },
    spec: { nodeName: node },
  });

  it("selects the pods by node field selector", () => {
    expect(podsOnNodePath("node a")).toBe("/api/v1/pods?fieldSelector=spec.nodeName%3Dnode%20a&limit=500");
  });

  it("evicts what it owns and leaves DaemonSet and static pods where they are", () => {
    const plan = drainPlan(
      [
        pod("web-0", "node-a"),
        pod("agent-0", "node-a", { ownerReferences: [{ kind: "DaemonSet", name: "agent", uid: "x" }] }),
        pod("etcd-a", "node-a", { annotations: { "kubernetes.io/config.mirror": "abc" } }),
        pod("web-1", "node-b"),
      ],
      "node-a",
    );
    expect(plan.evict.map(nameOf)).toEqual(["web-0"]);
    expect(plan.keep.map((entry) => [nameOf(entry.pod), entry.why])).toEqual([
      ["agent-0", "daemonSet"],
      ["etcd-a", "static"],
      ["web-1", "elsewhere"],
    ]);
  });

  it("asks for a policy/v1 eviction rather than a plain delete", () => {
    expect(evictionPath("team a", "web/0")).toBe("/api/v1/namespaces/team%20a/pods/web%2F0/eviction");
    expect(JSON.parse(evictionBody("team-a", "web-0"))).toEqual({
      apiVersion: "policy/v1",
      kind: "Eviction",
      metadata: { name: "web-0", namespace: "team-a" },
    });
  });
});

describe("ownership", () => {
  it("follows the ReplicaSet layer between a Deployment and its pods", () => {
    const replicaSets = [
      {
        metadata: {
          name: "web-abc",
          uid: "rs-1",
          ownerReferences: [{ kind: "Deployment", name: "web", uid: "dep-1" }],
        },
      },
      {
        metadata: {
          name: "api-xyz",
          uid: "rs-2",
          ownerReferences: [{ kind: "Deployment", name: "api", uid: "dep-2" }],
        },
      },
    ];
    const pods = [
      {
        metadata: {
          name: "web-abc-1",
          uid: "p1",
          ownerReferences: [{ kind: "ReplicaSet", name: "web-abc", uid: "rs-1" }],
        },
      },
      {
        metadata: {
          name: "api-xyz-1",
          uid: "p2",
          ownerReferences: [{ kind: "ReplicaSet", name: "api-xyz", uid: "rs-2" }],
        },
      },
      // Same name prefix, different owner: the uid is what keeps it out.
      {
        metadata: {
          name: "web-abc-2",
          uid: "p3",
          ownerReferences: [{ kind: "ReplicaSet", name: "other", uid: "rs-9" }],
        },
      },
    ];

    expect(podsOfOwner(pods, { kind: "Deployment", uid: "dep-1" }, replicaSets).map(uidOf)).toEqual(["p1"]);
    expect(podsOfOwner(pods, { kind: "ReplicaSet", uid: "rs-2" }, replicaSets).map(uidOf)).toEqual(["p2"]);
  });
});

describe("create", () => {
  it("posts to the collection the kind lives in, without the list's page size", () => {
    expect(createPath(descriptor("configmaps"), "team-a")).toBe("/api/v1/namespaces/team-a/configmaps");
    expect(createPath(descriptor("namespaces"), "team-a")).toBe("/api/v1/namespaces");
  });

  it("keeps the job's completion reading in the same place the table reads it", () => {
    expect(completionsText({ spec: { completions: 2 }, status: { succeeded: 2 } })).toBe("2/2");
  });
});
