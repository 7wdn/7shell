/**
 * The writes this workspace can make, as request shapes.
 *
 * Everything here is pure: the paths a write goes to, the bodies it carries, and which pods a drain
 * may and may not evict. The store is what sends them, which is what lets the rules — that a restart
 * is an annotation bump, that a DrainSet pod is not the drain's business, that a force delete is a
 * different URL — be tested without a cluster.
 *
 * Nothing here writes a URL by pasting a name into it: every segment goes through `encodeURIComponent`
 * so a name with a `/` in it (legal in a label, not in a path) cannot address another object.
 */
import { arrayAt, asObject, textAt, type K8sObject } from "@/components/k8s/objects";
import { listPathOf, selfPathOf, type ResourceDescriptor } from "@/components/k8s/resources";

function segment(value: string): string {
  return encodeURIComponent(value);
}

/** The annotation `kubectl rollout restart` stamps, which is what makes a rollout happen at all. */
export const RESTART_ANNOTATION = "kubectl.kubernetes.io/restartedAt";

/** Kinds whose pods are recreated by their controller, so a restart is a template change. */
const RESTARTABLE = new Set(["Deployment", "StatefulSet", "DaemonSet"]);

export function canRestart(desc: ResourceDescriptor): boolean {
  return RESTARTABLE.has(desc.kind);
}

/** A strategic merge patch: only the annotation moves, everything else stays as it was. */
export function restartPatch(nowIso: string): string {
  return JSON.stringify({ spec: { template: { metadata: { annotations: { [RESTART_ANNOTATION]: nowIso } } } } });
}

export function restartPath(desc: ResourceDescriptor, obj: K8sObject): string | null {
  if (!canRestart(desc)) {
    return null;
  }
  return selfPathOf(desc, obj);
}

export function restartContentType(): string {
  return "application/strategic-merge-patch+json";
}

export function scalePatch(replicas: number): string {
  return JSON.stringify({ spec: { replicas } });
}

export function scalePath(desc: ResourceDescriptor, obj: K8sObject): string {
  return `${selfPathOf(desc, obj)}/scale`;
}

/** A merge patch, because the `scale` subresource has no strategic form. */
export function scaleContentType(): string {
  return "application/merge-patch+json";
}

export function unschedulablePatch(unschedulable: boolean): string {
  return JSON.stringify({ spec: { unschedulable } });
}

/**
 * A delete, with the two things a force delete means.
 *
 * `gracePeriodSeconds=0` asks the kubelet to stop the container now rather than let it shut down;
 * `propagationPolicy=Background` deletes the children in the background instead of refusing while
 * they exist. Both are query parameters on the object's own URL — the API has no body for this.
 */
export function deletePath(selfPath: string, force: boolean): string {
  if (!force) {
    return selfPath;
  }
  const separator = selfPath.includes("?") ? "&" : "?";
  return `${selfPath}${separator}gracePeriodSeconds=0&propagationPolicy=Background`;
}

/** The pods running on one node, as the drain lists them. */
export function podsOnNodePath(nodeName: string): string {
  return `/api/v1/pods?fieldSelector=${segment(`spec.nodeName=${nodeName}`)}&limit=500`;
}

export function evictionPath(namespace: string, name: string): string {
  return `/api/v1/namespaces/${segment(namespace)}/pods/${segment(name)}/eviction`;
}

/** A `policy/v1` eviction: the request the API server's disruption budget actually answers. */
export function evictionBody(namespace: string, name: string): string {
  return JSON.stringify({ apiVersion: "policy/v1", kind: "Eviction", metadata: { name, namespace } });
}

/**
 * Whether a pod must stay when its node is drained.
 *
 * A DaemonSet's pod is on the node because the node exists; evicting it would only have the daemon
 * put one straight back. A static (mirror) pod is the kubelet's own, written from a file on that
 * machine, and the API server cannot evict something it does not own. Both are skipped rather than
 * failed, which is what `kubectl drain` reports too.
 */
export type DrainSkipReason = "daemonSet" | "static" | "elsewhere";

function skipReason(pod: K8sObject, nodeName: string): DrainSkipReason | null {
  if (textAt(pod, "spec", "nodeName") !== nodeName) {
    return "elsewhere";
  }
  if (textAt(pod, "metadata", "annotations", "kubernetes.io/config.mirror") !== "") {
    return "static";
  }
  const daemonSet = arrayAt(pod, "metadata", "ownerReferences").some((entry) => {
    const owner = asObject(entry);
    return owner !== null && textAt(owner, "kind") === "DaemonSet";
  });
  return daemonSet ? "daemonSet" : null;
}

export interface DrainPlan {
  /** The pods whose eviction the drain will ask for. */
  readonly evict: readonly K8sObject[];
  /** The pods it leaves alone, each with the reason a person reading the result would want. */
  readonly keep: readonly { readonly pod: K8sObject; readonly why: DrainSkipReason }[];
}

/** Which pods a drain of `nodeName` would try to evict, and which it would not. */
export function drainPlan(pods: readonly K8sObject[], nodeName: string): DrainPlan {
  const evict: K8sObject[] = [];
  const keep: { pod: K8sObject; why: DrainSkipReason }[] = [];
  for (const pod of pods) {
    const why = skipReason(pod, nodeName);
    if (why === null) {
      evict.push(pod);
    } else {
      keep.push({ pod, why });
    }
  }
  return { evict, keep };
}

/**
 * The pods a workload owns, following the ReplicaSet layer a Deployment inserts between itself and
 * its pods. Matching on the owner UID rather than on a name prefix is what keeps two workloads whose
 * names start the same way apart.
 */
export function podsOfOwner(
  pods: readonly K8sObject[],
  owner: { readonly kind: string; readonly uid: string },
  replicaSets: readonly K8sObject[],
): readonly K8sObject[] {
  if (owner.kind === "Deployment") {
    const owned = new Set(
      replicaSets
        .filter((set) =>
          arrayAt(set, "metadata", "ownerReferences").some((entry) => {
            const reference = asObject(entry);
            return reference !== null && textAt(reference, "uid") === owner.uid;
          }),
        )
        .map((set) => textAt(set, "metadata", "uid")),
    );
    return pods.filter((pod) =>
      arrayAt(pod, "metadata", "ownerReferences").some((entry) => {
        const reference = asObject(entry);
        return reference !== null && owned.has(textAt(reference, "uid"));
      }),
    );
  }
  return pods.filter((pod) =>
    arrayAt(pod, "metadata", "ownerReferences").some((entry) => {
      const reference = asObject(entry);
      return reference !== null && textAt(reference, "uid") === owner.uid;
    }),
  );
}

/** The collection a create posts to, with the namespace the new object is meant for. */
export function createPath(desc: ResourceDescriptor, namespace: string): string {
  const path = listPathOf(desc, desc.namespaced ? namespace : "");
  const query = path.indexOf("?");
  return query < 0 ? path : path.slice(0, query);
}
