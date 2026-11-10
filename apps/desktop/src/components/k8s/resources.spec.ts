/**
 * The resource table: the paths, and the shape of what it holds.
 *
 * An API path is the one place a name is pasted into a URL, so the encoding rules are asserted
 * rather than assumed — a workload whose name contains a slash is legal in a label and catastrophic
 * in a path. The rest is a check that the table itself stays coherent: every kind's label exists, no
 * key is claimed twice, and a cluster-wide kind never builds a namespaced URL.
 */
import { describe, expect, it } from "vitest";

import {
  OVERVIEW_KEY,
  RESOURCES,
  RESOURCE_GROUPS,
  basePathOf,
  collectionPath,
  descriptorOf,
  listPathOf,
  resourceGroups,
  selfPathOf,
  watchPathOf,
} from "@/components/k8s/resources";

describe("paths", () => {
  it("builds the core group under /api and every other group under /apis", () => {
    expect(basePathOf(descriptorOf("pods") ?? fail())).toBe("/api/v1");
    expect(basePathOf(descriptorOf("deployments") ?? fail())).toBe("/apis/apps/v1");
    expect(basePathOf(descriptorOf("ingresses") ?? fail())).toBe("/apis/networking.k8s.io/v1");
  });

  it("scopes a namespaced kind to its namespace and leaves a cluster-wide one alone", () => {
    expect(collectionPath(descriptorOf("pods") ?? fail(), "team-a")).toBe("/api/v1/namespaces/team-a/pods");
    expect(collectionPath(descriptorOf("pods") ?? fail(), "")).toBe("/api/v1/pods");
    expect(collectionPath(descriptorOf("nodes") ?? fail(), "team-a")).toBe("/api/v1/nodes");
    expect(listPathOf(descriptorOf("services") ?? fail(), "team-a")).toBe(
      "/api/v1/namespaces/team-a/services?limit=500",
    );
  });

  it("encodes a name rather than pasting it into the path", () => {
    const deployments = descriptorOf("deployments") ?? fail();
    expect(selfPathOf(deployments, { metadata: { name: "api/v2", namespace: "team a" } })).toBe(
      "/apis/apps/v1/namespaces/team%20a/deployments/api%2Fv2",
    );
  });

  it("watches from the version it was given, with bookmarks asked for", () => {
    const path = watchPathOf(descriptorOf("pods") ?? fail(), "default", "1234");
    expect(path).toBe("/api/v1/namespaces/default/pods?watch=true&allowWatchBookmarks=true&resourceVersion=1234");
  });
});

describe("the table", () => {
  it("names every kind with a message key and a label, and never twice", () => {
    const keys = new Set<string>();
    for (const descriptor of RESOURCES) {
      expect(descriptor.labelKey).toBe(`k8s.resources.${descriptor.key}`);
      expect(descriptor.columns.length).toBeGreaterThan(0);
      expect(keys.has(descriptor.key)).toBe(false);
      keys.add(descriptor.key);
    }
  });

  it("has a column key for every heading, so no heading falls back to its own key", () => {
    for (const descriptor of RESOURCES) {
      for (const column of descriptor.columns) {
        expect(column.headerKey).toBe(`k8s.columns.${column.key}`);
      }
    }
  });

  it("keeps the tree's groups in the order it draws them, with nothing empty", () => {
    const groups = resourceGroups();
    expect(groups.map((entry) => entry.group)).toEqual(
      RESOURCE_GROUPS.filter((group) => groups.some((e) => e.group === group)),
    );
    for (const entry of groups) {
      expect(entry.resources.length).toBeGreaterThan(0);
    }
  });

  it("answers nothing for a kind this build does not draw, rather than a guess", () => {
    expect(descriptorOf("widgets")).toBeNull();
    expect(descriptorOf(OVERVIEW_KEY)).toBeNull();
  });

  it("offers a terminal only where a shell can be opened, and a drain only on a node", () => {
    for (const descriptor of RESOURCES) {
      if (descriptor.actions.includes("terminal") || descriptor.actions.includes("logs")) {
        expect(descriptor.kind).toBe("Pod");
      }
      if (descriptor.actions.includes("drain") || descriptor.actions.includes("cordon")) {
        expect(descriptor.kind).toBe("Node");
      }
    }
  });

  it("hides a Secret's values in the YAML view and nothing else's", () => {
    expect(descriptorOf("secrets")?.redactData).toBe(true);
    expect(descriptorOf("configmaps")?.redactData).toBeUndefined();
  });
});

function fail(): never {
  throw new Error("the descriptor table lost a kind this build is supposed to draw");
}
