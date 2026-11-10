/**
 * Reading API objects that arrived as JSON.
 *
 * A Kubernetes object is a document the server owns, so nothing here types it: the fields this build
 * reads are narrowed out of `unknown` one level at a time. A missing field is an empty string rather
 * than a crash, because a column that shows nothing is a truthful answer for an object the cluster
 * has not finished filling in.
 */

// The two quantity readers this module needs; `quantity.ts` depends on nothing here, so this is not
// a cycle even though both live beside each other.
import { parseCpu as parseQuantityCpu, parseMemory as parseQuantityMemory } from "@/components/k8s/quantity";

/** One API object, as the server sent it. Keys are whatever the kind's schema says. */
export type K8sObject = Record<string, unknown>;

/** What a metrics reading is, for one pod or one node: millicores and bytes. */
export interface Usage {
  readonly cpu: number;
  readonly mem: number;
}

const NOTHING: Usage = { cpu: 0, mem: 0 };

export function asObject(value: unknown): K8sObject | null {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? (value as K8sObject) : null;
}

/** A field of an object, or `{}` when it is missing or is not an object. */
export function objectAt(value: unknown, ...keys: readonly string[]): K8sObject {
  let current: unknown = value;
  for (const key of keys) {
    const parent = asObject(current);
    if (!parent) {
      return {};
    }
    current = parent[key];
  }
  return asObject(current) ?? {};
}

/** An array field, or an empty list when it is missing or is not an array. */
export function arrayAt(value: unknown, ...keys: readonly string[]): readonly unknown[] {
  const parent = keys.length > 0 ? objectAt(value, ...keys.slice(0, -1)) : (asObject(value) ?? {});
  const last = keys[keys.length - 1];
  const found = last === undefined ? undefined : parent[last];
  return Array.isArray(found) ? found : [];
}

/** A string (or a number spelled as one) out of a document, or `""`. */
export function textAt(value: unknown, ...keys: readonly string[]): string {
  if (keys.length === 0) {
    return typeof value === "string" ? value : typeof value === "number" ? String(value) : "";
  }
  const parent = objectAt(value, ...keys.slice(0, -1));
  const found = parent[keys[keys.length - 1] ?? ""];
  return typeof found === "string" ? found : typeof found === "number" ? String(found) : "";
}

export function boolAt(value: unknown, ...keys: readonly string[]): boolean {
  const parent = objectAt(value, ...keys.slice(0, -1));
  return parent[keys[keys.length - 1] ?? ""] === true;
}

export function numberAt(value: unknown, ...keys: readonly string[]): number | null {
  const parent = objectAt(value, ...keys.slice(0, -1));
  const found = parent[keys[keys.length - 1] ?? ""];
  return typeof found === "number" && Number.isFinite(found) ? found : null;
}

// ── metadata ────────────────────────────────────────────────────

export function nameOf(obj: K8sObject): string {
  return textAt(obj, "metadata", "name");
}

export function namespaceOf(obj: K8sObject): string {
  return textAt(obj, "metadata", "namespace");
}

export function uidOf(obj: K8sObject): string {
  return textAt(obj, "metadata", "uid");
}

/** How a watch cache tells two objects apart: the UID when there is one, the qualified name otherwise. */
export function idOf(obj: K8sObject): string {
  const uid = uidOf(obj);
  if (uid !== "") {
    return uid;
  }
  const name = nameOf(obj);
  const namespace = namespaceOf(obj);
  return namespace === "" ? name : `${namespace}/${name}`;
}

export function createdAtOf(obj: K8sObject): string {
  return textAt(obj, "metadata", "creationTimestamp");
}

/** `kind/name` for every owner that named one, one per line. */
export function ownersText(obj: K8sObject): string {
  const lines = arrayAt(obj, "metadata", "ownerReferences")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null)
    .map((entry) => `${textAt(entry, "kind")}/${textAt(entry, "name")}`);
  return lines.length > 0 ? lines.join("\n") : "—";
}

/** A label or annotation map as `key=value` lines. */
export function mapText(value: unknown): string {
  const map = asObject(value);
  if (!map) {
    return "—";
  }
  const lines = Object.entries(map).map(
    ([key, entry]) => `${key}=${typeof entry === "string" ? entry : JSON.stringify(entry)}`,
  );
  return lines.length > 0 ? lines.join("\n") : "—";
}

// ── time ────────────────────────────────────────────────────────

/** How long ago a timestamp was, in the short shape a table has room for: `42s`, `9m`, `3h`, `2d`. */
export function age(iso: string, now: number = Date.now()): string {
  if (iso === "") {
    return "—";
  }
  const started = Date.parse(iso);
  return Number.isNaN(started) ? "—" : durationOf(Math.floor((now - started) / 1000));
}

/** How long an interval was, from a start and an end that may still be running. */
export function durationBetween(start: string, end?: string, now: number = Date.now()): string {
  if (start === "") {
    return "—";
  }
  const from = Date.parse(start);
  if (Number.isNaN(from)) {
    return "—";
  }
  const to = end === undefined || end === "" ? now : Date.parse(end);
  if (Number.isNaN(to)) {
    return "—";
  }
  return durationOf(Math.floor((to - from) / 1000));
}

function durationOf(seconds: number): string {
  if (seconds < 0) {
    return "—";
  }
  if (seconds < 60) {
    return `${seconds}s`;
  }
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return `${minutes}m`;
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return `${hours}h`;
  }
  return `${Math.floor(hours / 24)}d`;
}

// ── pods ────────────────────────────────────────────────────────

/**
 * The state a pod is in, as a table column shows it.
 *
 * The phase alone is not enough: a pod that is `Running` while one container waits on
 * `ImagePullBackOff` is the case somebody opens this table to find. So a waiting or terminated
 * container reason wins over the phase, which is the rule `kubectl get pods` follows.
 */
export function podStatus(obj: K8sObject): string {
  if (textAt(obj, "metadata", "deletionTimestamp") !== "") {
    return "Terminating";
  }
  const reason = textAt(obj, "status", "reason");
  if (reason !== "") {
    return reason;
  }
  for (const entry of arrayAt(obj, "status", "containerStatuses")) {
    const status = asObject(entry);
    if (!status) {
      continue;
    }
    const waiting = textAt(status, "state", "waiting", "reason");
    // `ContainerCreating` is what every pod is in for a moment; the phase already says `Pending`.
    if (waiting !== "" && waiting !== "ContainerCreating") {
      return waiting;
    }
    const terminated = textAt(status, "state", "terminated", "reason");
    if (terminated !== "" && terminated !== "Completed") {
      return terminated;
    }
  }
  return textAt(obj, "status", "phase");
}

/** `ready/total` across the pod's containers, which is what "2/2" in a list means. */
export function podReady(obj: K8sObject): string {
  const statuses = arrayAt(obj, "status", "containerStatuses")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null);
  const ready = statuses.filter((status) => boolAt(status, "ready")).length;
  return `${ready}/${statuses.length}`;
}

export function podRestarts(obj: K8sObject): number {
  return arrayAt(obj, "status", "containerStatuses").reduce<number>((total, entry) => {
    const status = asObject(entry);
    return total + (status ? (numberAt(status, "restartCount") ?? 0) : 0);
  }, 0);
}

/** The container names in a pod, in the order the manifest lists them. */
export function containerNamesOf(obj: K8sObject): string[] {
  return arrayAt(obj, "spec", "containers")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null)
    .map((entry) => textAt(entry, "name"))
    .filter((name) => name !== "");
}

export function imagesText(obj: K8sObject): string {
  const images = arrayAt(obj, "spec", "containers")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null)
    .map((entry) => textAt(entry, "image"))
    .filter((image) => image !== "");
  return images.length > 0 ? images.join("\n") : "—";
}

/** How a row should read: healthy, worth a look, or broken. Drives the row's tint. */
export type Tone = "" | "warn" | "err";

export function podTone(obj: K8sObject): Tone {
  if (textAt(obj, "metadata", "deletionTimestamp") !== "") {
    return "warn";
  }
  const status = podStatus(obj);
  if (status === "Running" || status === "Completed" || status === "Succeeded") {
    const statuses = arrayAt(obj, "status", "containerStatuses")
      .map((entry) => asObject(entry))
      .filter((entry): entry is K8sObject => entry !== null);
    if (status === "Running" && statuses.length > 0 && !statuses.every((entry) => boolAt(entry, "ready"))) {
      return "warn";
    }
    return "";
  }
  if (status === "Pending" || status === "ContainerCreating" || status === "PodInitializing") {
    return "warn";
  }
  return "err";
}

/** How many of a pod's containers asked for CPU or memory, summed. Used by the `%` columns. */
export function podRequests(obj: K8sObject): { readonly cpu: number; readonly mem: number } {
  return resourcesOf(obj, "requests");
}

export function podLimits(obj: K8sObject): { readonly cpu: number; readonly mem: number } {
  return resourcesOf(obj, "limits");
}

function resourcesOf(obj: K8sObject, field: "requests" | "limits"): Usage {
  let cpu = 0;
  let mem = 0;
  for (const entry of arrayAt(obj, "spec", "containers")) {
    const container = asObject(entry);
    if (!container) {
      continue;
    }
    const resources = objectAt(container, "resources", field);
    cpu += parseQuantityCpu(textAt(resources, "cpu"));
    mem += parseQuantityMemory(textAt(resources, "memory"));
  }
  return { cpu, mem };
}

/** What a job has finished, as `done/desired`. */
export function completionsText(obj: K8sObject): string {
  const succeeded = textAt(obj, "status", "succeeded");
  const desired = textAt(obj, "spec", "completions");
  const done = succeeded === "" ? "0" : succeeded;
  return `${done}/${desired === "" ? "1" : desired}`;
}

// ── workload replicas ───────────────────────────────────────────

export function replicasOf(obj: K8sObject): number {
  const spec = numberAt(obj, "spec", "replicas");
  if (spec !== null) {
    return spec;
  }
  // A DaemonSet has no `replicas`: its desired count is what the controller has observed.
  return numberAt(obj, "status", "desiredNumberScheduled") ?? 0;
}

export function readyReplicasOf(obj: K8sObject): number {
  const ready = numberAt(obj, "status", "readyReplicas");
  if (ready !== null) {
    return ready;
  }
  return numberAt(obj, "status", "numberReady") ?? 0;
}

/** `ready/desired` as a workload list shows it. */
export function replicaSummary(obj: K8sObject): string {
  return `${readyReplicasOf(obj)}/${replicasOf(obj)}`;
}

export function replicaTone(obj: K8sObject): Tone {
  const desired = replicasOf(obj);
  const ready = readyReplicasOf(obj);
  if (desired > 0 && ready < desired) {
    return ready === 0 ? "err" : "warn";
  }
  return "";
}

// ── services, ingress, storage, nodes ───────────────────────────

/** `name:port→nodePort/protocol` per port, which is how a service's ports are read at a glance. */
export function servicePorts(obj: K8sObject): string {
  const ports = arrayAt(obj, "spec", "ports").map((entry) => {
    const port = asObject(entry);
    if (!port) {
      return "";
    }
    const name = textAt(port, "name");
    const number = textAt(port, "port");
    const nodePort = textAt(port, "nodePort");
    const protocol = textAt(port, "protocol");
    const head = name === "" ? number : `${name}:${number}`;
    const tail = nodePort === "" ? "" : `→${nodePort}`;
    const suffix = protocol === "" || protocol === "TCP" ? "" : `/${protocol}`;
    return `${head}${tail}${suffix}`;
  });
  return ports.filter((port) => port !== "").join(" ") || "—";
}

export function ingressHosts(obj: K8sObject): string {
  const hosts = arrayAt(obj, "spec", "rules")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null)
    .map((entry) => {
      const host = textAt(entry, "host");
      return host === "" ? "*" : host;
    });
  return hosts.length > 0 ? hosts.join(", ") : "*";
}

/** The storage a claim or a volume holds, from `status.capacity.storage` or from `spec.resources`. */
export function capacityOf(obj: K8sObject): string {
  const status = textAt(obj, "status", "capacity", "storage");
  return status !== "" ? status : textAt(obj, "spec", "resources", "requests", "storage");
}

export function accessModesOf(obj: K8sObject): string {
  const modes = arrayAt(obj, "spec", "accessModes").filter((mode): mode is string => typeof mode === "string");
  return modes.length > 0 ? modes.join(", ") : "—";
}

export function storageClassOf(obj: K8sObject): string {
  const named = textAt(obj, "spec", "storageClassName");
  return named === "" ? "—" : named;
}

export function nodeReady(obj: K8sObject): boolean {
  return arrayAt(obj, "status", "conditions").some((entry) => {
    const condition = asObject(entry);
    return condition !== null && textAt(condition, "type") === "Ready" && textAt(condition, "status") === "True";
  });
}

/** What a node is for, from its `node-role.kubernetes.io/*` labels. */
export function nodeRoles(obj: K8sObject): string {
  const labels = objectAt(obj, "metadata", "labels");
  const roles = Object.keys(labels)
    .filter((label) => label.startsWith("node-role.kubernetes.io/"))
    .map((label) => label.slice("node-role.kubernetes.io/".length))
    .filter((role) => role !== "");
  return roles.length > 0 ? roles.sort().join(", ") : "—";
}

/** What a node can still hand out, in the units the table columns use. */
export function allocatableOf(obj: K8sObject): Usage {
  const allocatable = objectAt(obj, "status", "allocatable");
  return {
    cpu: parseQuantityCpu(textAt(allocatable, "cpu")),
    mem: parseQuantityMemory(textAt(allocatable, "memory")),
  };
}

/** `type=status` per condition, one per line. */
export function conditionsText(obj: K8sObject): string {
  const lines = arrayAt(obj, "status", "conditions")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null)
    .map((condition) => {
      const reason = textAt(condition, "reason");
      const head = `${textAt(condition, "type")}=${textAt(condition, "status")}`;
      return reason === "" ? head : `${head} (${reason})`;
    });
  return lines.length > 0 ? lines.join("\n") : "—";
}

/** The keys of a ConfigMap or a Secret — names only, which is all this build ever shows. */
export function keysOf(obj: K8sObject): string {
  const keys = new Set<string>();
  for (const field of ["data", "binaryData", "stringData"]) {
    for (const key of Object.keys(objectAt(obj, field))) {
      keys.add(key);
    }
  }
  return keys.size > 0 ? [...keys].sort().join(", ") : "—";
}

export function selectorText(obj: K8sObject): string {
  return mapText(objectAt(obj, "spec", "selector", "matchLabels"));
}

/** `key=value` pairs from a plain string map, for a job's or a cronjob's labels. */
export function mapOrDash(value: unknown): string {
  return mapText(value);
}

export function usageOrNothing(value: Usage | undefined | null): Usage {
  return value ?? NOTHING;
}
