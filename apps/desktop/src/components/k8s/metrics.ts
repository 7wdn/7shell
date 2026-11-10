/**
 * What the metrics server says each pod and node is using.
 *
 * The metrics API is a different group (`metrics.k8s.io/v1beta1`) and is present only where the
 * metrics server is installed. Its absence is not an error to report: a cluster without it simply
 * has no numbers, and every column that would show one shows `—` instead. That is why parsing is a
 * pair of pure functions here and the "is it missing" decision is made by the caller, which is the
 * only place that can tell a 404 from a connection failure.
 */
import { arrayAt, asObject, textAt, type K8sObject, type Usage } from "@/components/k8s/objects";
import { parseCpu, parseMemory } from "@/components/k8s/quantity";

/** The key a pod's usage is filed under: its namespace and name, the pair that identifies it. */
export function podUsageKey(namespace: string, name: string): string {
  return `${namespace}/${name}`;
}

function usageFrom(container: K8sObject): Usage {
  return {
    cpu: parseCpu(textAt(container, "usage", "cpu")),
    mem: parseMemory(textAt(container, "usage", "memory")),
  };
}

/** One entry per pod, with every container's usage added up. */
export function parsePodMetrics(body: unknown): Record<string, Usage> {
  const usage: Record<string, Usage> = {};
  for (const entry of arrayAt(body, "items")) {
    const pod = asObject(entry);
    if (!pod) {
      continue;
    }
    const name = textAt(pod, "metadata", "name");
    if (name === "") {
      continue;
    }
    let cpu = 0;
    let mem = 0;
    for (const containerEntry of arrayAt(pod, "containers")) {
      const container = asObject(containerEntry);
      if (!container) {
        continue;
      }
      const reading = usageFrom(container);
      cpu += reading.cpu;
      mem += reading.mem;
    }
    usage[podUsageKey(textAt(pod, "metadata", "namespace"), name)] = { cpu, mem };
  }
  return usage;
}

/** One entry per node, keyed by its name. */
export function parseNodeMetrics(body: unknown): Record<string, Usage> {
  const usage: Record<string, Usage> = {};
  for (const entry of arrayAt(body, "items")) {
    const node = asObject(entry);
    if (!node) {
      continue;
    }
    const name = textAt(node, "metadata", "name");
    if (name === "") {
      continue;
    }
    usage[name] = usageFrom(node);
  }
  return usage;
}

/** The sum of a set of readings, for a cluster-wide meter. */
export function totalUsage(readings: Readonly<Record<string, Usage>>): Usage {
  let cpu = 0;
  let mem = 0;
  for (const reading of Object.values(readings)) {
    cpu += reading.cpu;
    mem += reading.mem;
  }
  return { cpu, mem };
}
