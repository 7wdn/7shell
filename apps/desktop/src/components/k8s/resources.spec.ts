/**
 * The resource table: the paths, and the shape of what it holds.
 *
 * An API path is the one place a name is pasted into a URL, so the encoding rules are asserted
 * rather than assumed — a workload whose name contains a slash is legal in a label and catastrophic
 * in a path. The rest is a check that the table itself stays coherent: every kind's label exists, no
 * key is claimed twice, and a cluster-wide kind never builds a namespaced URL.
 */
import { afterEach, describe, expect, it } from "vitest";

import {
  CRD_KEY,
  OVERVIEW_KEY,
  RESOURCES,
  RESOURCE_GROUPS,
  basePathOf,
  clearCustomResourceDescriptors,
  collectionPath,
  crdKeyOf,
  crdShapeOf,
  customResourceDescriptor,
  customResourceDescriptors,
  descriptorOf,
  jsonPathValue,
  listPathOf,
  resourceGroups,
  selfPathOf,
  setCustomResourceDescriptors,
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

/**
 * The definitions a cluster sends are data, so the descriptor for what they introduce is built at
 * read time. These cases pin the parts that would otherwise fail quietly: the version picked, the
 * URL built from the plural rather than from the descriptor's own key, and the registry the tree
 * reads.
 */
const widgetCrd = {
  apiVersion: "apiextensions.k8s.io/v1",
  kind: "CustomResourceDefinition",
  metadata: { name: "widgets.example.com" },
  spec: {
    group: "example.com",
    names: { plural: "widgets", kind: "Widget" },
    scope: "Namespaced",
    versions: [
      { name: "v1alpha1", served: true, storage: false, additionalPrinterColumns: [] },
      {
        name: "v1",
        served: true,
        storage: true,
        additionalPrinterColumns: [
          { name: "Size", jsonPath: ".spec.size", type: "string" },
          { name: "Phase", jsonPath: ".status.phase" },
        ],
      },
    ],
  },
};

afterEach(() => {
  clearCustomResourceDescriptors("conn-a");
  clearCustomResourceDescriptors("conn-b");
});

describe("a custom kind", () => {
  it("reads the kind from the version the cluster stores, with that version's printer columns", () => {
    const shape = crdShapeOf(widgetCrd);
    expect(shape).not.toBeNull();
    expect(shape?.group).toBe("example.com");
    expect(shape?.version).toBe("v1");
    expect(shape?.plural).toBe("widgets");
    expect(shape?.kind).toBe("Widget");
    expect(shape?.namespaced).toBe(true);
    expect(shape?.printerColumns.map((column) => column.name)).toEqual(["Size", "Phase"]);
  });

  it("falls back to a served version when no version says it is stored", () => {
    const noStorage = {
      spec: { ...widgetCrd.spec, versions: [{ name: "v2", served: true, storage: false }] },
    };
    expect(crdShapeOf(noStorage)?.version).toBe("v2");
  });

  it("answers nothing for an object that is not a definition", () => {
    expect(crdShapeOf({ metadata: { name: "not-a-crd" } })).toBeNull();
    expect(crdShapeOf({ spec: { group: "example.com", names: {} } })).toBeNull();
  });

  it("builds cluster and namespaced collection paths from the plural, not the key", () => {
    const namespaced = customResourceDescriptor(widgetCrd) ?? fail();
    expect(namespaced.key).toBe("crd:widgets.example.com");
    expect(namespaced.apiVersion).toBe("example.com/v1");
    expect(collectionPath(namespaced, "team-a")).toBe("/apis/example.com/v1/namespaces/team-a/widgets");
    expect(listPathOf(namespaced, "team-a")).toBe("/apis/example.com/v1/namespaces/team-a/widgets?limit=500");
    expect(watchPathOf(namespaced, "", "9")).toBe(
      "/apis/example.com/v1/widgets?watch=true&allowWatchBookmarks=true&resourceVersion=9",
    );
    expect(selfPathOf(namespaced, { metadata: { name: "w a", namespace: "team-a" } })).toBe(
      "/apis/example.com/v1/namespaces/team-a/widgets/w%20a",
    );

    const clusterWide =
      customResourceDescriptor({
        spec: { ...widgetCrd.spec, scope: "Cluster" },
      }) ?? fail();
    expect(clusterWide.namespaced).toBe(false);
    expect(clusterWide.columns.map((column) => column.key)).toEqual(["name", "printer0", "printer1", "age"]);
    expect(listPathOf(clusterWide, "team-a")).toBe("/apis/example.com/v1/widgets?limit=500");
  });

  it("draws Name, printer columns and Age, and offers a delete", () => {
    const descriptor = customResourceDescriptor(widgetCrd) ?? fail();
    expect(descriptor.labelKey).toBe("k8s.resources.custom");
    expect(descriptor.kindLabel).toBe("Widget");
    expect(descriptor.columns.map((column) => column.key)).toEqual([
      "name",
      "namespace",
      "printer0",
      "printer1",
      "age",
    ]);
    expect(descriptor.columns.map((column) => column.headerKey)).toEqual([
      "k8s.columns.name",
      "k8s.columns.namespace",
      "Size",
      "Phase",
      "k8s.columns.age",
    ]);
    expect(descriptor.actions).toEqual(["delete"]);
    const size = descriptor.columns.find((column) => column.key === "printer0");
    expect(size?.text({ status: {}, spec: { size: "L" } })).toBe("L");
  });

  it("reads a printer column's value through the JSONPath subset and answers nothing on a miss", () => {
    const obj = { spec: { size: "L" }, status: { conditions: [{ type: "Ready" }] } };
    expect(jsonPathValue(obj, ".spec.size")).toBe("L");
    expect(jsonPathValue(obj, "status.conditions[0].type")).toBe("Ready");
    expect(jsonPathValue(obj, ".spec.missing")).toBe("");
    expect(jsonPathValue(obj, "spec")).toBe('{"size":"L"}');
  });

  it("offers a discovered kind to the tree and takes it away when the connection closes", () => {
    const descriptor = customResourceDescriptor(widgetCrd) ?? fail();
    setCustomResourceDescriptors("conn-a", [descriptor]);

    expect(descriptorOf(crdKeyOf(crdShapeOf(widgetCrd) ?? fail()))).toEqual(descriptor);
    const custom = resourceGroups().find((entry) => entry.group === "custom");
    expect(custom?.resources.map((entry) => entry.kindLabel)).toEqual(["Widget"]);

    clearCustomResourceDescriptors("conn-a");
    expect(descriptorOf(descriptor.key)).toBeNull();
    expect(customResourceDescriptors()).toEqual([]);
    expect(resourceGroups().some((entry) => entry.group === "custom")).toBe(false);
  });

  it("keeps one entry when two connections define the same kind", () => {
    const descriptor = customResourceDescriptor(widgetCrd) ?? fail();
    setCustomResourceDescriptors("conn-a", [descriptor]);
    setCustomResourceDescriptors("conn-b", [descriptor]);
    expect(customResourceDescriptors()).toHaveLength(1);

    clearCustomResourceDescriptors("conn-a");
    expect(customResourceDescriptors()).toHaveLength(1);
  });
});

describe("the definitions themselves", () => {
  it("are a cluster-wide kind under the apiextensions group", () => {
    const descriptor = descriptorOf(CRD_KEY) ?? fail();
    expect(descriptor.kind).toBe("CustomResourceDefinition");
    expect(descriptor.namespaced).toBe(false);
    expect(listPathOf(descriptor, "team-a")).toBe("/apis/apiextensions.k8s.io/v1/customresourcedefinitions?limit=500");
  });
});

function fail(): never {
  throw new Error("the descriptor table lost a kind this build is supposed to draw");
}
