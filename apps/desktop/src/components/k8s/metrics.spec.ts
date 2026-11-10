/**
 * Metrics parsing: what the metrics API says, in the units the columns divide by.
 *
 * A pod's usage is per container and a row is per pod, so the addition is the work. Getting it wrong
 * is invisible — a pod that reports one container's worth of CPU still draws a bar — which is why the
 * multi-container case is the one asserted.
 */
import { describe, expect, it } from "vitest";

import { parseNodeMetrics, parsePodMetrics, podUsageKey, totalUsage } from "@/components/k8s/metrics";

describe("pods", () => {
  it("adds every container's usage under the pod it belongs to", () => {
    const usage = parsePodMetrics({
      items: [
        {
          metadata: { namespace: "default", name: "web-0" },
          containers: [{ usage: { cpu: "250m", memory: "64Mi" } }, { usage: { cpu: "1", memory: "1Gi" } }],
        },
        { metadata: { namespace: "data", name: "db-0" }, containers: [{ usage: { cpu: "500m", memory: "2Gi" } }] },
      ],
    });

    expect(usage[podUsageKey("default", "web-0")]).toEqual({ cpu: 1250, mem: 64 * 1024 ** 2 + 1024 ** 3 });
    expect(usage[podUsageKey("data", "db-0")]).toEqual({ cpu: 500, mem: 2 * 1024 ** 3 });
  });

  it("skips entries with no name rather than filing them under an empty key", () => {
    expect(parsePodMetrics({ items: [{ containers: [{ usage: { cpu: "1m" } }] }] })).toEqual({});
  });

  it("answers nothing for a body that is not a list", () => {
    expect(parsePodMetrics(null)).toEqual({});
    expect(parsePodMetrics({ kind: "Status" })).toEqual({});
  });
});

describe("nodes", () => {
  it("files each node's usage under its name", () => {
    const usage = parseNodeMetrics({
      items: [{ metadata: { name: "node-a" }, usage: { cpu: "1500m", memory: "8Gi" } }],
    });
    expect(usage["node-a"]).toEqual({ cpu: 1500, mem: 8 * 1024 ** 3 });
  });
});

describe("totals", () => {
  it("sums the cluster's usage for the overview's meters", () => {
    expect(
      totalUsage({
        a: { cpu: 500, mem: 1024 },
        b: { cpu: 1500, mem: 2048 },
      }),
    ).toEqual({ cpu: 2000, mem: 3072 });
    expect(totalUsage({})).toEqual({ cpu: 0, mem: 0 });
  });
});
