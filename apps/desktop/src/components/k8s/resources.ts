/**
 * The resource table: one entry per kind, and everything the tree, the list and the detail drawer
 * need to draw it.
 *
 * A cluster's API is uniform — a collection URL, a list, a watch, objects with `metadata`, `spec` and
 * `status` — and the differences between kinds are the *readings* each one is worth showing. Those
 * differences live here rather than in a component per kind, so adding HorizontalPodAutoscalers is a
 * table entry and not a sixth list view. Labels are i18n keys, never text: this file has no language.
 *
 * The paths are built from the descriptor too. Nothing anywhere else pastes a name into a URL.
 */
import {
  accessModesOf,
  age,
  arrayAt,
  asObject,
  boolAt,
  capacityOf,
  containerNamesOf,
  completionsText,
  createdAtOf,
  imagesText,
  ingressHosts,
  keysOf,
  mapText,
  nameOf,
  namespaceOf,
  nodeReady,
  nodeRoles,
  numberAt,
  objectAt,
  ownersText,
  podLimits,
  podReady,
  podRequests,
  podRestarts,
  podStatus,
  podTone,
  replicaSummary,
  replicaTone,
  selectorText,
  servicePorts,
  storageClassOf,
  textAt,
  type K8sObject,
  type Tone,
  type Usage,
} from "@/components/k8s/objects";
import { formatCpu, formatMemory, parseCpu, parseMemory, percent } from "@/components/k8s/quantity";

/** The groups the tree draws, in the order it draws them. */
export type ResourceGroup = "workloads" | "network" | "config" | "storage" | "cluster" | "rbac";

/**
 * What the tree calls the overview.
 *
 * Not a resource key, so it cannot be mistaken for one: the pane compares the selection against this
 * to decide whether it draws the overview or a list.
 */
export const OVERVIEW_KEY = "overview";

export const RESOURCE_GROUPS: readonly ResourceGroup[] = [
  "workloads",
  "network",
  "config",
  "storage",
  "cluster",
  "rbac",
];

/** The icons the tree knows how to draw. A name rather than a component, so this file stays data. */
export type ResourceIcon =
  | "box"
  | "layers"
  | "database"
  | "copy"
  | "clock"
  | "network"
  | "globe"
  | "file"
  | "lock"
  | "drive"
  | "server"
  | "folder"
  | "bell"
  | "shield"
  | "cable"
  | "package"
  | "gauge";

/**
 * A column, as data.
 *
 * `text` is what the cell shows; `sortKey` is what the heading sorts by when the text is not itself
 * comparable — a CPU reading, a memory reading, a restart count. Without it, ordering falls back to
 * `numericValueOf`, which reads percentages, ages and ratios out of the text.
 */
export interface ColumnDef {
  /** Stable identity for the sort state, and the last segment of the heading's i18n key. */
  readonly key: string;
  readonly headerKey: string;
  readonly text: (obj: K8sObject, usage?: Usage | null) => string;
  readonly sortKey?: (obj: K8sObject, usage?: Usage | null) => number;
  /** Whether the toolbar's filter reads this column. The name always is read. */
  readonly searchable?: boolean;
}

export interface DetailFieldDef {
  readonly key: string;
  readonly labelKey: string;
  readonly value: (obj: K8sObject) => string;
}

export interface DetailSectionDef {
  readonly key: string;
  readonly labelKey: string;
  readonly fields: readonly DetailFieldDef[];
}

/** What a row offers, in the order the action buttons are drawn. */
export type ResourceAction = "logs" | "terminal" | "viewPods" | "restart" | "scale" | "cordon" | "drain" | "delete";

export interface ResourceDescriptor {
  readonly key: string;
  readonly kind: string;
  /** `v1` or `group/version`, exactly as the object's `apiVersion` spells it. */
  readonly apiVersion: string;
  readonly group: ResourceGroup;
  readonly namespaced: boolean;
  readonly labelKey: string;
  readonly icon: ResourceIcon;
  readonly columns: readonly ColumnDef[];
  readonly actions: readonly ResourceAction[];
  readonly detail: readonly DetailSectionDef[];
  /** Which metrics the row can carry. Absent means the kind has none. */
  readonly metrics?: "pod" | "node";
  /** The row's health, when the kind has one worth tinting. */
  readonly tone?: (obj: K8sObject) => Tone;
  /** Whether the YAML view hides the object's values because the object is a secret. */
  readonly redactData?: boolean;
  readonly createTemplate?: (namespace: string) => string;
}

// ── paths ───────────────────────────────────────────────────────

/** Where a kind's collection lives: `/api/v1` for the core group, `/apis/<group>/<version>` otherwise. */
export function basePathOf(desc: ResourceDescriptor): string {
  const [first, second] = desc.apiVersion.split("/");
  return second === undefined ? `/api/${first}` : `/apis/${first}/${second}`;
}

function segment(value: string): string {
  return encodeURIComponent(value);
}

/** The collection itself, with no query. This is where a create posts and where a watch starts. */
export function collectionPath(desc: ResourceDescriptor, namespace: string): string {
  const base = `${basePathOf(desc)}/${desc.key}`;
  return desc.namespaced && namespace !== ""
    ? `${basePathOf(desc)}/namespaces/${segment(namespace)}/${desc.key}`
    : base;
}

/** The collection as a list reads it. The limit is a page, not a cap: the server says what it left. */
export function listPathOf(desc: ResourceDescriptor, namespace: string): string {
  return `${collectionPath(desc, namespace)}?limit=500`;
}

/**
 * The collection as a watch reads it.
 *
 * The bookmark flag is what keeps a quiet resource's `resourceVersion` moving, so a connection that
 * has been idle for an hour is not made to relist from nothing when it wakes up.
 */
export function watchPathOf(desc: ResourceDescriptor, namespace: string, resourceVersion: string): string {
  const query = ["watch=true", "allowWatchBookmarks=true", `resourceVersion=${encodeURIComponent(resourceVersion)}`];
  return `${collectionPath(desc, namespace)}?${query.join("&")}`;
}

/** One object's own URL. */
export function selfPathOf(desc: ResourceDescriptor, obj: K8sObject): string {
  return `${collectionPath(desc, namespaceOf(obj))}/${segment(nameOf(obj))}`;
}

// ── columns and sections, as builders ───────────────────────────

function column(
  key: string,
  text: ColumnDef["text"],
  options: { readonly sortKey?: ColumnDef["sortKey"]; readonly searchable?: boolean } = {},
): ColumnDef {
  return { key, headerKey: `k8s.columns.${key}`, text, ...options };
}

/** The name column, which every list has and every filter reads. */
function nameColumn(): ColumnDef {
  return column("name", (obj) => nameOf(obj), { searchable: true });
}

function namespaceColumn(): ColumnDef {
  return column("namespace", (obj) => namespaceOf(obj), { searchable: true });
}

function ageColumn(): ColumnDef {
  return column("age", (obj) => age(createdAtOf(obj)));
}

function usageColumn(key: "cpu" | "memory", read: (usage: Usage) => string, pick: (usage: Usage) => number): ColumnDef {
  return column(key, (_obj, usage) => (usage ? read(usage) : "—"), {
    sortKey: (_obj, usage) => (usage ? pick(usage) : -1),
  });
}

function cpuColumn(): ColumnDef {
  return usageColumn(
    "cpu",
    (usage) => formatCpu(usage.cpu),
    (usage) => usage.cpu,
  );
}

function memoryColumn(): ColumnDef {
  return usageColumn(
    "memory",
    (usage) => formatMemory(usage.mem),
    (usage) => usage.mem,
  );
}

function field(key: string, value: (obj: K8sObject) => string): DetailFieldDef {
  return { key, labelKey: `k8s.detail.fields.${key}`, value };
}

/** The section every object has. */
function metadataSection(): DetailSectionDef {
  return {
    key: "metadata",
    labelKey: "k8s.detail.sections.metadata",
    fields: [
      field("name", (obj) => nameOf(obj)),
      field("namespace", (obj) => namespaceOf(obj) || "—"),
      field("uid", (obj) => textAt(obj, "metadata", "uid") || "—"),
      field("created", (obj) => createdAtOf(obj) || "—"),
      field("labels", (obj) => mapText(asObject(obj.metadata)?.labels)),
      field("annotations", (obj) => mapText(asObject(obj.metadata)?.annotations)),
      field("owner", (obj) => ownersText(obj)),
    ],
  };
}

function section(key: string, fields: readonly DetailFieldDef[]): DetailSectionDef {
  return { key, labelKey: `k8s.detail.sections.${key}`, fields };
}

/**
 * A boolean detail field, spelled the way the manifest spells it.
 *
 * Not "yes" and "no": these values are the object's own, they appear in the YAML view beside this
 * one, and `true` is what somebody copying the field back expects to read.
 */
function yesNo(value: boolean): string {
  return String(value);
}

// ── descriptors ─────────────────────────────────────────────────

const POD = {
  key: "pods",
  kind: "Pod",
  apiVersion: "v1",
  group: "workloads" as const,
  namespaced: true,
  labelKey: "k8s.resources.pods",
  icon: "box" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("ready", (obj) => podReady(obj), { searchable: true }),
    column("status", (obj) => podStatus(obj), { searchable: true }),
    column("restarts", (obj) => String(podRestarts(obj)), { sortKey: (obj) => podRestarts(obj) }),
    cpuColumn(),
    memoryColumn(),
    column("cpuRequested", (obj, usage) => (usage ? percent(usage.cpu, podRequests(obj).cpu) : "—")),
    column("memoryRequested", (obj, usage) => (usage ? percent(usage.mem, podRequests(obj).mem) : "—")),
    column("ip", (obj) => textAt(obj, "status", "podIP"), { searchable: true }),
    column("node", (obj) => textAt(obj, "spec", "nodeName"), { searchable: true }),
    ageColumn(),
  ],
  actions: ["logs", "terminal", "delete"] as const,
  metrics: "pod" as const,
  tone: podTone,
  detail: [
    metadataSection(),
    section("pod", [
      field("status", (obj) => podStatus(obj)),
      field("phase", (obj) => textAt(obj, "status", "phase") || "—"),
      field("node", (obj) => textAt(obj, "spec", "nodeName") || "—"),
      field("podIp", (obj) => textAt(obj, "status", "podIP") || "—"),
      field("qos", (obj) => textAt(obj, "status", "qosClass") || "—"),
      field("containers", (obj) => containerNamesOf(obj).join("\n") || "—"),
      field("images", (obj) => imagesText(obj)),
      field("requests", (obj) => requestedText(obj)),
      field("conditions", (obj) => conditions(obj)),
    ]),
  ],
  createTemplate: (namespace) => `apiVersion: v1
kind: Pod
metadata:
  name: example
  namespace: ${namespace}
spec:
  containers:
    - name: main
      image: nginx:1.27
      resources:
        requests:
          cpu: 100m
          memory: 128Mi
`,
} satisfies ResourceDescriptor;

const DEPLOYMENT = {
  key: "deployments",
  kind: "Deployment",
  apiVersion: "apps/v1",
  group: "workloads" as const,
  namespaced: true,
  labelKey: "k8s.resources.deployments",
  icon: "layers" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("ready", (obj) => replicaSummary(obj)),
    column("upToDate", (obj) => String(numberAt(obj, "status", "updatedReplicas") ?? 0)),
    column("available", (obj) => String(numberAt(obj, "status", "availableReplicas") ?? 0)),
    ageColumn(),
  ],
  actions: ["restart", "scale", "viewPods", "delete"] as const,
  tone: replicaTone,
  detail: [
    metadataSection(),
    section("workload", [
      field("replicas", (obj) => replicaSummary(obj)),
      field("strategy", (obj) => textAt(obj, "spec", "strategy", "type") || "—"),
      field("selector", (obj) => selectorText(obj)),
      field("images", (obj) => imagesText(obj)),
      field("conditions", (obj) => conditions(obj)),
    ]),
  ],
  createTemplate: (namespace) => `apiVersion: apps/v1
kind: Deployment
metadata:
  name: example
  namespace: ${namespace}
spec:
  replicas: 1
  selector:
    matchLabels:
      app: example
  template:
    metadata:
      labels:
        app: example
    spec:
      containers:
        - name: main
          image: nginx:1.27
          resources:
            requests:
              cpu: 100m
              memory: 128Mi
`,
} satisfies ResourceDescriptor;

const STATEFUL_SET = {
  key: "statefulsets",
  kind: "StatefulSet",
  apiVersion: "apps/v1",
  group: "workloads" as const,
  namespaced: true,
  labelKey: "k8s.resources.statefulsets",
  icon: "database" as const,
  columns: [nameColumn(), namespaceColumn(), column("ready", (obj) => replicaSummary(obj)), ageColumn()],
  actions: ["restart", "scale", "viewPods", "delete"] as const,
  tone: replicaTone,
  detail: [
    metadataSection(),
    section("workload", [
      field("replicas", (obj) => replicaSummary(obj)),
      field("selector", (obj) => selectorText(obj)),
      field("images", (obj) => imagesText(obj)),
      field("conditions", (obj) => conditions(obj)),
    ]),
  ],
} satisfies ResourceDescriptor;

const DAEMON_SET = {
  key: "daemonsets",
  kind: "DaemonSet",
  apiVersion: "apps/v1",
  group: "workloads" as const,
  namespaced: true,
  labelKey: "k8s.resources.daemonsets",
  icon: "copy" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("ready", (obj) => replicaSummary(obj)),
    column("upToDate", (obj) => String(numberAt(obj, "status", "updatedNumberScheduled") ?? 0)),
    column("available", (obj) => String(numberAt(obj, "status", "numberAvailable") ?? 0)),
    ageColumn(),
  ],
  actions: ["restart", "viewPods", "delete"] as const,
  tone: replicaTone,
  detail: [
    metadataSection(),
    section("workload", [
      field("replicas", (obj) => replicaSummary(obj)),
      field("selector", (obj) => selectorText(obj)),
      field("images", (obj) => imagesText(obj)),
      field("conditions", (obj) => conditions(obj)),
    ]),
  ],
} satisfies ResourceDescriptor;

const JOB = {
  key: "jobs",
  kind: "Job",
  apiVersion: "batch/v1",
  group: "workloads" as const,
  namespaced: true,
  labelKey: "k8s.resources.jobs",
  icon: "clock" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("completions", (obj) => completionsText(obj)),
    column("active", (obj) => String(numberAt(obj, "status", "active") ?? 0)),
    ageColumn(),
  ],
  actions: ["viewPods", "delete"] as const,
  detail: [
    metadataSection(),
    section("job", [
      field("completions", (obj) => completionsText(obj)),
      field("parallelism", (obj) => textAt(obj, "spec", "parallelism") || "—"),
      field("conditions", (obj) => conditions(obj)),
      field("selector", (obj) => selectorText(obj)),
    ]),
  ],
  createTemplate: (namespace) => `apiVersion: batch/v1
kind: Job
metadata:
  name: example
  namespace: ${namespace}
spec:
  template:
    spec:
      restartPolicy: Never
      containers:
        - name: main
          image: busybox:1.36
          command: ["sh", "-c", "echo hello"]
`,
} satisfies ResourceDescriptor;

const CRON_JOB = {
  key: "cronjobs",
  kind: "CronJob",
  apiVersion: "batch/v1",
  group: "workloads" as const,
  namespaced: true,
  labelKey: "k8s.resources.cronjobs",
  icon: "clock" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("schedule", (obj) => textAt(obj, "spec", "schedule"), { searchable: true }),
    column("suspend", (obj) => yesNo(boolAt(obj, "spec", "suspend"))),
    column("active", (obj) => String(arrayAt(obj, "status", "active").length)),
    column("lastSchedule", (obj) => age(textAt(obj, "status", "lastScheduleTime"))),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [
    metadataSection(),
    section("cronJob", [
      field("schedule", (obj) => textAt(obj, "spec", "schedule") || "—"),
      field("suspend", (obj) => yesNo(boolAt(obj, "spec", "suspend"))),
      field("lastSchedule", (obj) => age(textAt(obj, "status", "lastScheduleTime"))),
      field("selector", (obj) => selectorText(obj)),
    ]),
  ],
  createTemplate: (namespace) => `apiVersion: batch/v1
kind: CronJob
metadata:
  name: example
  namespace: ${namespace}
spec:
  schedule: "*/5 * * * *"
  jobTemplate:
    spec:
      template:
        spec:
          restartPolicy: Never
          containers:
            - name: main
              image: busybox:1.36
              command: ["sh", "-c", "date"]
`,
} satisfies ResourceDescriptor;

const REPLICA_SET = {
  key: "replicasets",
  kind: "ReplicaSet",
  apiVersion: "apps/v1",
  group: "workloads" as const,
  namespaced: true,
  labelKey: "k8s.resources.replicasets",
  icon: "copy" as const,
  columns: [nameColumn(), namespaceColumn(), column("ready", (obj) => replicaSummary(obj)), ageColumn()],
  actions: ["delete"] as const,
  tone: replicaTone,
  detail: [metadataSection(), section("workload", [field("replicas", (obj) => replicaSummary(obj))])],
} satisfies ResourceDescriptor;

const SERVICE = {
  key: "services",
  kind: "Service",
  apiVersion: "v1",
  group: "network" as const,
  namespaced: true,
  labelKey: "k8s.resources.services",
  icon: "network" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("type", (obj) => textAt(obj, "spec", "type") || "ClusterIP", { searchable: true }),
    column("clusterIp", (obj) => textAt(obj, "spec", "clusterIP") || "—"),
    column("ports", (obj) => servicePorts(obj), { searchable: true }),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [
    metadataSection(),
    section("service", [
      field("type", (obj) => textAt(obj, "spec", "type") || "ClusterIP"),
      field("clusterIp", (obj) => textAt(obj, "spec", "clusterIP") || "—"),
      field("ports", (obj) => servicePorts(obj)),
      field("selector", (obj) => mapText(asObject(obj.spec)?.selector)),
    ]),
  ],
  createTemplate: (namespace) => `apiVersion: v1
kind: Service
metadata:
  name: example
  namespace: ${namespace}
spec:
  selector:
    app: example
  ports:
    - port: 80
      targetPort: 80
`,
} satisfies ResourceDescriptor;

const INGRESS = {
  key: "ingresses",
  kind: "Ingress",
  apiVersion: "networking.k8s.io/v1",
  group: "network" as const,
  namespaced: true,
  labelKey: "k8s.resources.ingresses",
  icon: "globe" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("class", (obj) => textAt(obj, "spec", "ingressClassName") || "—"),
    column("hosts", (obj) => ingressHosts(obj), { searchable: true }),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [
    metadataSection(),
    section("ingress", [
      field("class", (obj) => textAt(obj, "spec", "ingressClassName") || "—"),
      field("hosts", (obj) => ingressHosts(obj)),
      field("tls", (obj) => tlsText(obj)),
    ]),
  ],
  createTemplate: (namespace) => `apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: example
  namespace: ${namespace}
spec:
  rules:
    - host: example.test
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: example
                port:
                  number: 80
`,
} satisfies ResourceDescriptor;

const NETWORK_POLICY = {
  key: "networkpolicies",
  kind: "NetworkPolicy",
  apiVersion: "networking.k8s.io/v1",
  group: "network" as const,
  namespaced: true,
  labelKey: "k8s.resources.networkpolicies",
  icon: "shield" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("selector", (obj) => compact(mapText(objectAt(obj, "spec", "podSelector", "matchLabels")))),
    column("policyTypes", (obj) => arrayAt(obj, "spec", "policyTypes").join(", ") || "—"),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [
    metadataSection(),
    section("networkPolicy", [
      field("selector", (obj) => selectorText(obj)),
      field("policyTypes", (obj) => arrayAt(obj, "spec", "policyTypes").join(", ") || "—"),
      field("ingressRules", (obj) => String(arrayAt(obj, "spec", "ingress").length)),
      field("egressRules", (obj) => String(arrayAt(obj, "spec", "egress").length)),
    ]),
  ],
} satisfies ResourceDescriptor;

const ENDPOINTS = {
  key: "endpoints",
  kind: "Endpoints",
  apiVersion: "v1",
  group: "network" as const,
  namespaced: true,
  labelKey: "k8s.resources.endpoints",
  icon: "cable" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("endpoints", (obj) => compact(endpointsText(obj)), { searchable: true }),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [metadataSection(), section("endpoints", [field("endpoints", (obj) => compact(endpointsText(obj)))])],
} satisfies ResourceDescriptor;

const CONFIG_MAP = {
  key: "configmaps",
  kind: "ConfigMap",
  apiVersion: "v1",
  group: "config" as const,
  namespaced: true,
  labelKey: "k8s.resources.configmaps",
  icon: "file" as const,
  columns: [nameColumn(), namespaceColumn(), column("keys", (obj) => keysOf(obj), { searchable: true }), ageColumn()],
  actions: ["delete"] as const,
  detail: [metadataSection(), section("config", [field("keys", (obj) => keysOf(obj))])],
  createTemplate: (namespace) => `apiVersion: v1
kind: ConfigMap
metadata:
  name: example
  namespace: ${namespace}
data:
  key: value
`,
} satisfies ResourceDescriptor;

const SECRET = {
  key: "secrets",
  kind: "Secret",
  apiVersion: "v1",
  group: "config" as const,
  namespaced: true,
  labelKey: "k8s.resources.secrets",
  icon: "lock" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("type", (obj) => textAt(obj, "type") || "Opaque"),
    column("keys", (obj) => keysOf(obj), { searchable: true }),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  redactData: true,
  detail: [
    metadataSection(),
    section("config", [field("type", (obj) => textAt(obj, "type") || "Opaque"), field("keys", (obj) => keysOf(obj))]),
  ],
} satisfies ResourceDescriptor;

const SERVICE_ACCOUNT = {
  key: "serviceaccounts",
  kind: "ServiceAccount",
  apiVersion: "v1",
  group: "config" as const,
  namespaced: true,
  labelKey: "k8s.resources.serviceaccounts",
  icon: "file" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("secrets", (obj) => String(arrayAt(obj, "secrets").length)),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [metadataSection(), section("config", [field("secrets", (obj) => String(arrayAt(obj, "secrets").length))])],
} satisfies ResourceDescriptor;

const PVC = {
  key: "persistentvolumeclaims",
  kind: "PersistentVolumeClaim",
  apiVersion: "v1",
  group: "storage" as const,
  namespaced: true,
  labelKey: "k8s.resources.persistentvolumeclaims",
  icon: "drive" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("status", (obj) => textAt(obj, "status", "phase"), { searchable: true }),
    column("capacity", (obj) => capacityOf(obj) || "—"),
    column("accessModes", (obj) => accessModesOf(obj)),
    column("class", (obj) => storageClassOf(obj)),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [
    metadataSection(),
    section("storage", [
      field("status", (obj) => textAt(obj, "status", "phase") || "—"),
      field("capacity", (obj) => capacityOf(obj) || "—"),
      field("accessModes", (obj) => accessModesOf(obj)),
      field("storageClass", (obj) => storageClassOf(obj)),
      field("volume", (obj) => textAt(obj, "spec", "volumeName") || "—"),
    ]),
  ],
} satisfies ResourceDescriptor;

const PV = {
  key: "persistentvolumes",
  kind: "PersistentVolume",
  apiVersion: "v1",
  group: "storage" as const,
  namespaced: false,
  labelKey: "k8s.resources.persistentvolumes",
  icon: "database" as const,
  columns: [
    nameColumn(),
    column("status", (obj) => textAt(obj, "status", "phase"), { searchable: true }),
    column("capacity", (obj) => capacityOf(obj) || "—"),
    column("accessModes", (obj) => accessModesOf(obj)),
    column("reclaim", (obj) => textAt(obj, "spec", "persistentVolumeReclaimPolicy") || "—"),
    column("class", (obj) => textAt(obj, "spec", "storageClassName") || "—"),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [
    metadataSection(),
    section("storage", [
      field("status", (obj) => textAt(obj, "status", "phase") || "—"),
      field("capacity", (obj) => capacityOf(obj) || "—"),
      field("accessModes", (obj) => accessModesOf(obj)),
      field("reclaim", (obj) => textAt(obj, "spec", "persistentVolumeReclaimPolicy") || "—"),
      field("claim", (obj) => claimText(obj)),
    ]),
  ],
} satisfies ResourceDescriptor;

const STORAGE_CLASS = {
  key: "storageclasses",
  kind: "StorageClass",
  apiVersion: "storage.k8s.io/v1",
  group: "storage" as const,
  namespaced: false,
  labelKey: "k8s.resources.storageclasses",
  icon: "drive" as const,
  columns: [
    nameColumn(),
    column("provisioner", (obj) => textAt(obj, "provisioner") || "—"),
    column("reclaim", (obj) => textAt(obj, "reclaimPolicy") || "Delete"),
    column("bindingMode", (obj) => textAt(obj, "volumeBindingMode") || "Immediate"),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [
    metadataSection(),
    section("storage", [
      field("provisioner", (obj) => textAt(obj, "provisioner") || "—"),
      field("reclaim", (obj) => textAt(obj, "reclaimPolicy") || "Delete"),
      field("volumeBindingMode", (obj) => textAt(obj, "volumeBindingMode") || "Immediate"),
      field("allowExpansion", (obj) => yesNo(boolAt(obj, "allowVolumeExpansion"))),
      field("parameters", (obj) => mapText(asObject(obj.parameters))),
    ]),
  ],
} satisfies ResourceDescriptor;

const NODE = {
  key: "nodes",
  kind: "Node",
  apiVersion: "v1",
  group: "cluster" as const,
  namespaced: false,
  labelKey: "k8s.resources.nodes",
  icon: "server" as const,
  columns: [
    nameColumn(),
    column("status", (obj) => (nodeReady(obj) ? "Ready" : "NotReady"), { searchable: true }),
    column("roles", (obj) => nodeRoles(obj), { searchable: true }),
    column("version", (obj) => textAt(obj, "status", "nodeInfo", "kubeletVersion") || "—"),
    column("cpuUsage", (_obj, usage) => (usage ? formatCpu(usage.cpu) : "—"), {
      sortKey: (_obj, usage) => (usage ? usage.cpu : -1),
    }),
    column("cpuUsagePercent", (obj, usage) => (usage ? percent(usage.cpu, nodeAllocatableCpu(obj)) : "—")),
    column("memoryUsage", (_obj, usage) => (usage ? formatMemory(usage.mem) : "—"), {
      sortKey: (_obj, usage) => (usage ? usage.mem : -1),
    }),
    column("memoryUsagePercent", (obj, usage) => (usage ? percent(usage.mem, nodeAllocatableMemory(obj)) : "—")),
    column("schedulable", (obj) => yesNo(!boolAt(obj, "spec", "unschedulable"))),
    ageColumn(),
  ],
  actions: ["cordon", "drain", "viewPods"] as const,
  metrics: "node" as const,
  tone: (obj) => (nodeReady(obj) ? "" : "err"),
  detail: [
    metadataSection(),
    section("node", [
      field("status", (obj) => (nodeReady(obj) ? "Ready" : "NotReady")),
      field("roles", (obj) => nodeRoles(obj)),
      field("kubelet", (obj) => textAt(obj, "status", "nodeInfo", "kubeletVersion") || "—"),
      field("os", (obj) => textAt(obj, "status", "nodeInfo", "osImage") || "—"),
      field("allocatable", (obj) => allocatableText(obj)),
      field("podCidr", (obj) => textAt(obj, "spec", "podCIDR") || "—"),
      field("conditions", (obj) => conditions(obj)),
    ]),
  ],
} satisfies ResourceDescriptor;

const NAMESPACE = {
  key: "namespaces",
  kind: "Namespace",
  apiVersion: "v1",
  group: "cluster" as const,
  namespaced: false,
  labelKey: "k8s.resources.namespaces",
  icon: "folder" as const,
  columns: [nameColumn(), column("status", (obj) => textAt(obj, "status", "phase"), { searchable: true }), ageColumn()],
  actions: ["viewPods", "delete"] as const,
  detail: [metadataSection(), section("namespace", [field("status", (obj) => textAt(obj, "status", "phase") || "—")])],
  createTemplate: () => `apiVersion: v1
kind: Namespace
metadata:
  name: example
`,
} satisfies ResourceDescriptor;

const EVENT = {
  key: "events",
  kind: "Event",
  apiVersion: "v1",
  group: "cluster" as const,
  namespaced: true,
  labelKey: "k8s.resources.events",
  icon: "bell" as const,
  columns: [
    column("reason", (obj) => textAt(obj, "reason"), { searchable: true }),
    column("type", (obj) => textAt(obj, "type") || "Normal", { searchable: true }),
    namespaceColumn(),
    column("object", (obj) => involvedObject(obj), { searchable: true }),
    column("count", (obj) => String(numberAt(obj, "count") ?? 1)),
    column("lastSeen", (obj) => age(textAt(obj, "lastTimestamp") || createdAtOf(obj))),
  ],
  actions: [] as const,
  detail: [
    metadataSection(),
    section("event", [
      field("type", (obj) => textAt(obj, "type") || "Normal"),
      field("reason", (obj) => textAt(obj, "reason") || "—"),
      field("object", (obj) => involvedObject(obj)),
      field("message", (obj) => textAt(obj, "message") || "—"),
      field("count", (obj) => String(numberAt(obj, "count") ?? 1)),
      field("lastSeen", (obj) => age(textAt(obj, "lastTimestamp") || createdAtOf(obj))),
    ]),
  ],
} satisfies ResourceDescriptor;

const HPA = {
  key: "horizontalpodautoscalers",
  kind: "HorizontalPodAutoscaler",
  apiVersion: "autoscaling/v2",
  group: "workloads" as const,
  namespaced: true,
  labelKey: "k8s.resources.horizontalpodautoscalers",
  icon: "gauge" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("target", (obj) => scaleTarget(obj)),
    column("targets", (obj) => hpaTargets(obj)),
    column("minReplicas", (obj) => String(numberAt(obj, "spec", "minReplicas") ?? 1)),
    column("maxReplicas", (obj) => String(numberAt(obj, "spec", "maxReplicas") ?? 1)),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [
    metadataSection(),
    section("autoscaler", [
      field("target", (obj) => scaleTarget(obj)),
      field("targets", (obj) => hpaTargets(obj)),
      field("replicas", (obj) => String(numberAt(obj, "status", "currentReplicas") ?? 0)),
      field("minReplicas", (obj) => String(numberAt(obj, "spec", "minReplicas") ?? 1)),
      field("maxReplicas", (obj) => String(numberAt(obj, "spec", "maxReplicas") ?? 1)),
    ]),
  ],
} satisfies ResourceDescriptor;

const ROLE = {
  key: "roles",
  kind: "Role",
  apiVersion: "rbac.authorization.k8s.io/v1",
  group: "rbac" as const,
  namespaced: true,
  labelKey: "k8s.resources.roles",
  icon: "shield" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("rules", (obj) => String(arrayAt(obj, "rules").length)),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [metadataSection(), section("access", [field("rules", (obj) => rulesText(obj))])],
} satisfies ResourceDescriptor;

const ROLE_BINDING = {
  key: "rolebindings",
  kind: "RoleBinding",
  apiVersion: "rbac.authorization.k8s.io/v1",
  group: "rbac" as const,
  namespaced: true,
  labelKey: "k8s.resources.rolebindings",
  icon: "shield" as const,
  columns: [
    nameColumn(),
    namespaceColumn(),
    column("role", (obj) => textAt(obj, "roleRef", "name") || "—"),
    column("subjects", (obj) => String(arrayAt(obj, "subjects").length)),
    ageColumn(),
  ],
  actions: ["delete"] as const,
  detail: [
    metadataSection(),
    section("access", [
      field("role", (obj) => `${textAt(obj, "roleRef", "kind")}/${textAt(obj, "roleRef", "name")}`),
      field("subjects", (obj) => subjectsText(obj)),
    ]),
  ],
} satisfies ResourceDescriptor;

/** Every kind this build draws a list for, in the order the tree groups them. */
export const RESOURCES: readonly ResourceDescriptor[] = [
  POD,
  DEPLOYMENT,
  STATEFUL_SET,
  DAEMON_SET,
  JOB,
  CRON_JOB,
  REPLICA_SET,
  HPA,
  SERVICE,
  INGRESS,
  NETWORK_POLICY,
  ENDPOINTS,
  CONFIG_MAP,
  SECRET,
  SERVICE_ACCOUNT,
  PVC,
  PV,
  STORAGE_CLASS,
  NODE,
  NAMESPACE,
  EVENT,
  ROLE,
  ROLE_BINDING,
];

const BY_KEY: Readonly<Record<string, ResourceDescriptor>> = Object.fromEntries(
  RESOURCES.map((desc) => [desc.key, desc]),
);

/** The descriptor for a resource key, or `null` for a key this build does not draw. */
export function descriptorOf(key: string): ResourceDescriptor | null {
  return BY_KEY[key] ?? null;
}

/** The tree's groups, each with the kinds it holds, in table order. */
export function resourceGroups(): readonly {
  readonly group: ResourceGroup;
  readonly resources: readonly ResourceDescriptor[];
}[] {
  return RESOURCE_GROUPS.map((group) => ({
    group,
    resources: RESOURCES.filter((desc) => desc.group === group),
  })).filter((entry) => entry.resources.length > 0);
}

// ── shared text helpers used by the table above ─────────────────

function conditions(obj: K8sObject): string {
  const lines = arrayAt(obj, "status", "conditions")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null)
    .map((condition) => {
      const type = textAt(condition, "type");
      const status = textAt(condition, "status");
      const reason = textAt(condition, "reason");
      return reason === "" ? `${type}=${status}` : `${type}=${status} (${reason})`;
    });
  return lines.length > 0 ? lines.join("\n") : "—";
}

function requestedText(obj: K8sObject): string {
  const requests = podRequests(obj);
  const limits = podLimits(obj);
  return `requests ${formatCpu(requests.cpu)} / ${formatMemory(requests.mem)} — limits ${formatCpu(limits.cpu)} / ${formatMemory(limits.mem)}`;
}

function nodeAllocatableCpu(obj: K8sObject): number {
  return allocatable(obj).cpu;
}

function nodeAllocatableMemory(obj: K8sObject): number {
  return allocatable(obj).mem;
}

function allocatable(obj: K8sObject): Usage {
  return {
    cpu: parseCpu(textAt(obj, "status", "allocatable", "cpu")),
    mem: parseMemory(textAt(obj, "status", "allocatable", "memory")),
  };
}

function allocatableText(obj: K8sObject): string {
  const total = allocatable(obj);
  return `${formatCpu(total.cpu)} / ${formatMemory(total.mem)}`;
}

function claimText(obj: K8sObject): string {
  const namespace = textAt(obj, "spec", "claimRef", "namespace");
  const name = textAt(obj, "spec", "claimRef", "name");
  return namespace === "" && name === "" ? "—" : `${namespace}/${name}`;
}

function tlsText(obj: K8sObject): string {
  const hosts = arrayAt(obj, "spec", "tls")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null)
    .flatMap((entry) => arrayAt(entry, "hosts").filter((host): host is string => typeof host === "string"));
  return hosts.length > 0 ? hosts.join(", ") : "—";
}

function endpointsText(obj: K8sObject): string {
  const addresses: string[] = [];
  for (const subset of arrayAt(obj, "subsets")) {
    const block = asObject(subset);
    if (!block) {
      continue;
    }
    const ports = arrayAt(block, "ports")
      .map((entry) => asObject(entry))
      .filter((entry): entry is K8sObject => entry !== null)
      .map((entry) => textAt(entry, "port"))
      .filter((port) => port !== "");
    for (const address of arrayAt(block, "addresses")) {
      const ip = asObject(address) ? textAt(address, "ip") : "";
      if (ip === "") {
        continue;
      }
      if (ports.length === 0) {
        addresses.push(ip);
      } else {
        addresses.push(...ports.map((port) => `${ip}:${port}`));
      }
    }
  }
  return addresses.join(", ");
}

function involvedObject(obj: K8sObject): string {
  const kind = textAt(obj, "involvedObject", "kind");
  const name = textAt(obj, "involvedObject", "name");
  return kind === "" && name === "" ? "—" : `${kind}/${name}`;
}

function scaleTarget(obj: K8sObject): string {
  const kind = textAt(obj, "spec", "scaleTargetRef", "kind");
  const name = textAt(obj, "spec", "scaleTargetRef", "name");
  return kind === "" && name === "" ? "—" : `${kind}/${name}`;
}

/** Current over target for each metric an autoscaler watches, `cpu 40%/80%` shaped. */
function hpaTargets(obj: K8sObject): string {
  const current = new Map<string, string>();
  for (const entry of arrayAt(obj, "status", "currentMetrics")) {
    const metric = asObject(entry);
    if (!metric) {
      continue;
    }
    const resource = asObject(metric.resource);
    if (!resource) {
      continue;
    }
    const name = textAt(resource, "name");
    const utilization = numberAt(resource, "current", "averageUtilization");
    const value = textAt(resource, "current", "averageValue");
    current.set(name, utilization !== null ? `${utilization}%` : value || "?");
  }
  const pairs = arrayAt(obj, "spec", "metrics").flatMap((entry) => {
    const metric = asObject(entry);
    if (!metric) {
      return [];
    }
    const resource = asObject(metric.resource);
    if (!resource) {
      return [];
    }
    const name = textAt(resource, "name");
    const utilization = numberAt(resource, "target", "averageUtilization");
    const value = textAt(resource, "target", "averageValue");
    const target = utilization !== null ? `${utilization}%` : value || "?";
    return [`${name} ${current.get(name) ?? "?"}/${target}`];
  });
  return pairs.length > 0 ? pairs.join(", ") : "—";
}

function rulesText(obj: K8sObject): string {
  const lines = arrayAt(obj, "rules").map((entry) => {
    const rule = asObject(entry);
    if (!rule) {
      return "";
    }
    const groups = arrayAt(rule, "apiGroups")
      .filter((group): group is string => typeof group === "string")
      .map((group) => group || "core")
      .join(",");
    const resources = arrayAt(rule, "resources")
      .filter((resource): resource is string => typeof resource === "string")
      .join(",");
    const verbs = arrayAt(rule, "verbs")
      .filter((verb): verb is string => typeof verb === "string")
      .join(",");
    return `${groups}/${resources} → ${verbs}`;
  });
  return lines.filter((line) => line !== "").join("\n") || "—";
}

function subjectsText(obj: K8sObject): string {
  const lines = arrayAt(obj, "subjects")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null)
    .map((subject) => {
      const kind = textAt(subject, "kind");
      const name = textAt(subject, "name");
      const namespace = textAt(subject, "namespace");
      return namespace === "" ? `${kind}/${name}` : `${kind}/${namespace}:${name}`;
    });
  return lines.length > 0 ? lines.join("\n") : "—";
}

/** A detail value with its newlines flattened, for a column that draws one line. */
function compact(value: string): string {
  return value.replace(/\n/g, ", ");
}
