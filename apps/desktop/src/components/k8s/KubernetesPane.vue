<script setup lang="ts">
/**
 * The cluster workspace: a tree of kinds on the left, one list on the right, and the drawers that
 * open from a row.
 *
 * The tab owns one connection. It is opened when the pane is first drawn and let go when the pane
 * goes away, because a cluster connection with nothing on screen is a socket nobody can see or stop.
 * The tab keeps its `connectionId` field for the workspace's own bookkeeping; what this pane uses is
 * the store, which knows the same connection and everything following it.
 *
 * Every list is followed rather than read once, so what is on screen is what the cluster says now.
 * The overview is the exception: it is a set of readings rather than a subscription, so it is polled
 * every thirty seconds and only while this tab is the one on screen.
 */
import { LoaderCircle, RefreshCw, ShipWheel } from "@lucide/vue";
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { computed, onBeforeUnmount, ref, useId, watch } from "vue";
import { useI18n } from "vue-i18n";

import K8sActionPrompt from "@/components/k8s/K8sActionPrompt.vue";
import K8sCreateDialog from "@/components/k8s/K8sCreateDialog.vue";
import K8sDetailDrawer from "@/components/k8s/K8sDetailDrawer.vue";
import K8sExecDialog from "@/components/k8s/K8sExecDialog.vue";
import K8sLogsDrawer from "@/components/k8s/K8sLogsDrawer.vue";
import K8sOverviewView from "@/components/k8s/K8sOverviewView.vue";
import K8sResourceTable from "@/components/k8s/K8sResourceTable.vue";
import K8sTree from "@/components/k8s/K8sTree.vue";
import {
  arrayAt,
  asObject,
  boolAt,
  containerNamesOf,
  createdAtOf,
  idOf,
  nameOf,
  namespaceOf,
  textAt,
  type K8sObject,
  type Usage,
} from "@/components/k8s/objects";
import { OVERVIEW_KEY, descriptorOf, type ResourceAction, type ResourceDescriptor } from "@/components/k8s/resources";
import { useToast } from "@/composables/useToast";
import { describeShellError } from "@/i18n/errors";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { useK8sStore } from "@/stores/k8s";
import { type KubernetesTab, useTerminalsStore } from "@/stores/terminals";

const props = defineProps<{ tab: KubernetesTab; active: boolean }>();

const { t } = useI18n();
const { toast } = useToast();
const store = useK8sStore();
const terminals = useTerminalsStore();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const namespaceListId = useId();

const selected = ref<string>(OVERVIEW_KEY);
const namespace = ref(props.tab.session.kubernetes?.namespace ?? "");

const connection = computed(() => store.connectionOf(props.tab.session.id));
const connId = computed(() => store.connectionIdOf(props.tab.session.id));
const descriptor = computed(() =>
  selected.value === OVERVIEW_KEY ? null : descriptorOf(selected.value, connId.value ?? undefined),
);
const namespaced = computed(() => descriptor.value?.namespaced ?? false);

const namespaceOptions = computed(() => {
  const names = new Set(connection.value?.namespaces ?? []);
  const configured = props.tab.session.kubernetes?.namespace ?? "";
  if (configured !== "") {
    names.add(configured);
  }
  return [...names].sort();
});

const connectionFailure = computed(() => connection.value?.failure ?? null);

const rows = computed(() =>
  connId.value !== null && descriptor.value !== null
    ? store.rowsOf(connId.value, descriptor.value.key, namespace.value)
    : [],
);

/** What a row is using, for the kinds whose columns read metrics. */
function usageOf(obj: K8sObject): Usage | null {
  if (connId.value === null || descriptor.value === null) {
    return null;
  }
  if (descriptor.value.metrics === "pod") {
    return store.usageOfPod(connId.value, namespaceOf(obj), nameOf(obj));
  }
  return descriptor.value.metrics === "node" ? store.usageOfNode(connId.value, nameOf(obj)) : null;
}

const loading = computed(() =>
  connId.value !== null && descriptor.value !== null
    ? store.isLoading(connId.value, descriptor.value.key, namespace.value)
    : false,
);

const listFailure = computed(() =>
  connId.value !== null && descriptor.value !== null
    ? store.failureOf(connId.value, descriptor.value.key, namespace.value)
    : null,
);

const watchFailure = computed(() =>
  connId.value !== null && descriptor.value !== null
    ? store.watchFailureOf(connId.value, descriptor.value.key, namespace.value)
    : null,
);

const overview = computed(() => (connId.value === null ? null : (store.overviews[connId.value] ?? null)));
const overviewFailure = computed(() => (connId.value === null ? null : (store.overviewFailures[connId.value] ?? null)));
const metrics = computed(() =>
  connId.value === null ? { pods: {}, nodes: {}, available: true, failure: null } : store.metricsOf(connId.value),
);
const clusterUsage = computed(() => (connId.value === null ? { cpu: 0, mem: 0 } : store.clusterUsage(connId.value)));

/** A text the list starts filtered by, when a row sent somebody to another kind's list. */
const presetFilter = ref("");

// ── the connection ────────────────────────────────────────────

void store.connect(props.tab.session);

/**
 * Follows whatever the tree and the namespace picker currently say.
 *
 * The previous selection is let go before the new one is taken, so one connection does not carry a
 * subscription for every kind somebody clicked through on the way to this one.
 */
watch(
  () => [connId.value, selected.value, namespace.value] as const,
  (current, previous) => {
    const [id, key, ns] = current;
    if (previous !== undefined) {
      const [oldId, oldKey, oldNs] = previous;
      if (oldId !== null && oldKey !== OVERVIEW_KEY) {
        store.unsubscribe(oldId, oldKey, oldNs);
      }
    }
    if (id === null || key === OVERVIEW_KEY) {
      return;
    }
    store.subscribe(id, key, ns);
    void store.loadMetrics(id, ns);
  },
  { immediate: true },
);

// ── the overview's own clock ──────────────────────────────────

let poll: number | null = null;

function stopPolling(): void {
  if (poll !== null) {
    window.clearInterval(poll);
    poll = null;
  }
}

/**
 * Reads the overview now, and every thirty seconds while this tab is on screen.
 *
 * Not while it is behind another tab, and not while the window is minimised: a poll nobody is looking
 * at is a request per half minute for every cluster somebody left open.
 */
function syncPolling(): void {
  stopPolling();
  const id = connId.value;
  if (!props.active || id === null) {
    return;
  }
  void store.loadOverview(id);
  poll = window.setInterval(() => {
    if (document.visibilityState === "hidden") {
      return;
    }
    const current = connId.value;
    if (current !== null) {
      void store.loadOverview(current);
    }
  }, 30_000);
}

watch([() => props.active, connId], syncPolling, { immediate: true });

onBeforeUnmount(() => {
  stopPolling();
  const id = connId.value;
  if (id !== null && selected.value !== OVERVIEW_KEY) {
    store.unsubscribe(id, selected.value, namespace.value);
  }
  void store.disconnectSession(props.tab.session.id);
});

// ── the detail drawer ─────────────────────────────────────────

/** The object on show, by identity, so a watch event updates the drawer rather than a stale copy. */
const detailId = ref<string | null>(null);
const detailObject = computed(() => {
  if (detailId.value === null) {
    return null;
  }
  return rows.value.find((obj) => idOf(obj) === detailId.value) ?? null;
});

const events = ref<readonly K8sObject[]>([]);
const eventsLoading = ref(false);
const eventsFailure = ref<ShellError | null>(null);
let eventsGeneration = 0;

/** A pod's own events, which is where "why is it pending" is answered. */
async function loadEvents(pod: K8sObject): Promise<void> {
  const mine = ++eventsGeneration;
  events.value = [];
  eventsFailure.value = null;
  const id = connId.value;
  if (id === null) {
    return;
  }
  eventsLoading.value = true;
  const path = `/api/v1/namespaces/${encodeURIComponent(namespaceOf(pod))}/events?fieldSelector=${encodeURIComponent(
    `involvedObject.name=${nameOf(pod)}`,
  )}&limit=200`;
  try {
    const body = await store.request(id, { method: "get", path });
    if (mine !== eventsGeneration) {
      return;
    }
    events.value = arrayAt(body, "items")
      .map((entry) => asObject(entry))
      .filter((entry): entry is K8sObject => entry !== null)
      .sort((left, right) =>
        (textAt(left, "lastTimestamp") || createdAtOf(left)).localeCompare(
          textAt(right, "lastTimestamp") || createdAtOf(right),
        ),
      );
  } catch (error) {
    if (mine === eventsGeneration) {
      eventsFailure.value = asShellError(error, "k8s_events");
    }
  } finally {
    if (mine === eventsGeneration) {
      eventsLoading.value = false;
    }
  }
}

function openDetail(obj: K8sObject): void {
  detailId.value = idOf(obj);
  events.value = [];
  eventsFailure.value = null;
  if (descriptor.value?.kind === "Pod") {
    void loadEvents(obj);
  }
}

function closeDetail(): void {
  detailId.value = null;
  eventsGeneration += 1;
  events.value = [];
  eventsFailure.value = null;
}

// ── the logs drawer ───────────────────────────────────────────

const logsTarget = ref<K8sObject | null>(null);

// ── the prompts ───────────────────────────────────────────────

type PromptKind = "delete" | "restart" | "scale" | "cordon" | "uncordon" | "drain";

const prompt = ref<{ readonly kind: PromptKind; readonly object: K8sObject } | null>(null);
const promptBusy = ref(false);

const execTarget = ref<K8sObject | null>(null);
const execBusy = ref(false);
const execFailure = ref<ShellError | null>(null);

const createOpen = ref(false);
const creating = ref(false);
const createFailure = ref<ShellError | null>(null);

const promptOpen = computed(() => prompt.value !== null);

/**
 * What a kind is called in a sentence.
 *
 * A kind the cluster defined has no message key — it did not exist when this build was made — so the
 * cluster's own word is used and everything around it stays translated.
 */
function kindName(desc: ResourceDescriptor): string {
  return desc.kindLabel ?? t(desc.labelKey);
}

function askFor(kind: PromptKind, obj: K8sObject): void {
  if (connId.value === null || descriptor.value === null) {
    return;
  }
  prompt.value = { kind, object: obj };
}

/** A write that changes one workload or one node, dispatched from the prompt. */
async function approve(answer: { readonly force: boolean; readonly replicas: number }): Promise<void> {
  const target = prompt.value;
  const desc = descriptor.value;
  const id = connId.value;
  if (target === null || desc === null || id === null) {
    return;
  }
  const object = target.object;
  promptBusy.value = true;
  try {
    switch (target.kind) {
      case "delete":
        await store.deleteObject(id, desc, object, answer.force);
        toast(t("k8s.result.deleted", { kind: kindName(desc), name: nameOf(object) }), "confirmation");
        break;
      case "restart":
        await store.restartWorkload(id, desc, object);
        toast(t("k8s.result.restarted", { kind: kindName(desc), name: nameOf(object) }), "confirmation");
        break;
      case "scale":
        await store.scaleWorkload(id, desc, object, answer.replicas);
        toast(
          t("k8s.result.scaled", {
            kind: kindName(desc),
            name: nameOf(object),
            count: String(answer.replicas),
          }),
          "confirmation",
        );
        break;
      case "cordon":
      case "uncordon": {
        const unschedulable = target.kind === "cordon";
        await store.setUnschedulable(id, nameOf(object), unschedulable);
        toast(
          t(unschedulable ? "k8s.result.cordoned" : "k8s.result.uncordoned", { name: nameOf(object) }),
          "confirmation",
        );
        break;
      }
      case "drain": {
        const outcome = await store.drainNode(id, nameOf(object));
        toast(
          t("k8s.result.drained", {
            name: nameOf(object),
            evicted: String(outcome.evicted),
            skipped: String(outcome.skipped),
          }) +
            (outcome.failures.length > 0
              ? ` ${t("k8s.result.drainFailed", { count: String(outcome.failures.length) })}`
              : ""),
          outcome.failures.length > 0 ? "failure" : "confirmation",
        );
        break;
      }
      default:
        break;
    }
    prompt.value = null;
    store.refresh(id, desc.key, namespace.value);
  } catch (error) {
    toast(describeShellError(translate, asShellError(error, "k8s_action")).message, "failure");
  } finally {
    promptBusy.value = false;
  }
}

/** What a row's action button asks for. */
function onRowAction(obj: K8sObject, action: ResourceAction): void {
  switch (action) {
    case "delete":
    case "restart":
    case "scale":
    case "drain":
      askFor(action, obj);
      return;
    case "cordon":
      askFor(boolAt(obj, "spec", "unschedulable") ? "uncordon" : "cordon", obj);
      return;
    case "viewPods": {
      presetFilter.value = nameOf(obj);
      selected.value = "pods";
      return;
    }
    case "files":
      startFiles(obj);
      return;
    default:
      return;
  }
}

// ── pod terminals ─────────────────────────────────────────────

function startTerminal(obj: K8sObject): void {
  if (connId.value === null) {
    return;
  }
  if (!store.isRunning(obj)) {
    toast(t("k8s.exec.notRunning"), "refusal");
    return;
  }
  const containers = arrayAt(obj, "spec", "containers")
    .map((entry) => asObject(entry))
    .filter((entry): entry is K8sObject => entry !== null)
    .map((entry) => textAt(entry, "name"))
    .filter((name) => name !== "");
  execFailure.value = null;
  if (containers.length === 1) {
    void openShell(obj, containers[0] ?? "");
    return;
  }
  if (containers.length === 0) {
    toast(t("k8s.exec.noContainer"), "refusal");
    return;
  }
  execTarget.value = obj;
}

async function openShell(pod: K8sObject, container: string): Promise<void> {
  const id = connId.value;
  if (id === null || container === "") {
    return;
  }
  execBusy.value = true;
  execFailure.value = null;
  try {
    await store.openExec(props.tab.session, id, pod, container);
    execTarget.value = null;
  } catch (error) {
    const failure = asShellError(error, "k8s_exec_open");
    if (execTarget.value === null) {
      // No dialog is open to carry the reason, so it is said where the person is looking.
      toast(describeShellError(translate, failure).message, "failure");
    } else {
      execFailure.value = failure;
    }
  } finally {
    execBusy.value = false;
  }
}

// ── pod files ─────────────────────────────────────────────────

/**
 * The pod whose files are waiting on a container pick, and the container picked so far.
 *
 * A pod with one container is answered without asking — a menu with one entry decides nothing —
 * which is why the target is only set when there is a choice to make.
 */
const filesTarget = ref<K8sObject | null>(null);
const filesChosen = ref("");
const filesBusy = ref(false);

const filesContainers = computed(() => (filesTarget.value === null ? [] : containerNamesOf(filesTarget.value)));

/**
 * Opens the file browser for a pod, or asks which container when it holds several.
 *
 * The gate is the same one the row's button obeys, and it is repeated here because a click can
 * arrive from a row that was drawn before the pod stopped: the answer is a toast rather than a
 * browser that would exec into a container nobody promised was there.
 */
function startFiles(obj: K8sObject): void {
  if (connId.value === null) {
    return;
  }
  if (!store.isRunning(obj)) {
    toast(t("k8s.files.notRunning"), "refusal");
    return;
  }
  const containers = containerNamesOf(obj);
  if (containers.length === 0) {
    toast(t("k8s.files.noContainer"), "refusal");
    return;
  }
  if (containers.length === 1) {
    void openFilesBrowser(obj, containers[0] ?? "");
    return;
  }
  filesChosen.value = containers[0] ?? "";
  filesTarget.value = obj;
}

async function openFilesBrowser(pod: K8sObject, container: string): Promise<void> {
  const id = connId.value;
  if (id === null || container === "") {
    return;
  }
  filesBusy.value = true;
  try {
    // The store reports a failure and leaves no tab behind. The question is answered either way,
    // so the picker closes: there is no reason line in it that a retry from inside could act on.
    await terminals.openPodFiles(props.tab.session, id, namespaceOf(pod), nameOf(pod), container, props.tab.id);
  } finally {
    filesBusy.value = false;
    filesTarget.value = null;
  }
}

function closeFiles(): void {
  filesTarget.value = null;
  filesChosen.value = "";
}

/** The dialog closing without a pick: the same thing as pressing Cancel. */
function onFilesOpenChange(value: boolean): void {
  if (!value) {
    closeFiles();
  }
}

// ── creating ──────────────────────────────────────────────────

function openCreate(): void {
  createFailure.value = null;
  createOpen.value = true;
}

/** The name a manifest gives its object, for the receipt. Nothing is validated against it. */
function manifestName(manifest: string): string {
  const found = /^\s*name:\s*(\S+)\s*$/m.exec(manifest);
  return found?.[1] ?? "";
}

async function create(manifest: string): Promise<void> {
  const id = connId.value;
  const desc = descriptor.value;
  if (id === null || desc === null) {
    return;
  }
  creating.value = true;
  createFailure.value = null;
  try {
    // The store relists the collection it posted to, so the new object appears without the pane
    // asking for a second read of the same list.
    await store.createObject(id, desc, namespace.value, manifest);
    toast(t("k8s.result.created", { kind: kindName(desc), name: manifestName(manifest) }), "confirmation");
    createOpen.value = false;
  } catch (error) {
    createFailure.value = asShellError(error, "k8s_create");
  } finally {
    creating.value = false;
  }
}

// ── what the tree and the dialogs change ──────────────────────

/**
 * A kind picked in the tree.
 *
 * The preset filter goes with it: it belonged to the row that sent somebody to another list, and a
 * filter left behind would read as a list with rows missing.
 */
function choose(key: string): void {
  presetFilter.value = "";
  selected.value = key;
}

function closePrompt(): void {
  prompt.value = null;
}

function closeExec(): void {
  execTarget.value = null;
  execFailure.value = null;
}

// ── retrying a failed connection ──────────────────────────────

async function retry(): Promise<void> {
  await store.reconnect(props.tab.session);
}
</script>

<template>
  <section class="flex min-h-0 flex-1 flex-col">
    <header class="flex shrink-0 items-center gap-2 border-b border-border px-3 py-[var(--panel-header-y)]">
      <ShipWheel :size="15" class="shrink-0 text-muted-foreground" />
      <h2 class="shrink-0 text-sm font-medium">{{ props.tab.session.name }}</h2>
      <span class="truncate font-mono text-[11px] text-muted-foreground">
        {{ props.tab.session.kubernetes?.context || t("k8s.endpointDefault") }}
      </span>
      <span
        v-if="connection?.version"
        class="rounded-sm border border-border px-1 font-mono text-[11px] text-muted-foreground"
      >
        {{ connection.version }}
      </span>

      <span class="ml-auto flex items-center gap-2 text-[11px] text-muted-foreground">
        <LoaderCircle v-if="connection?.phase === 'connecting'" :size="12" class="animate-spin" />
        <button
          v-if="connection?.phase === 'error'"
          type="button"
          class="flex items-center gap-1 rounded-md border border-border px-2 py-0.5 transition-colors hover:bg-accent"
          @click="retry()"
        >
          <RefreshCw :size="11" />
          {{ t("k8s.retry") }}
        </button>
        <span v-if="overview" class="font-mono">{{
          t("k8s.overview.updated", { time: store.overviewAge(connId ?? "") })
        }}</span>
      </span>
    </header>

    <div
      v-if="connection?.phase === 'error' && connectionFailure"
      role="alert"
      class="shrink-0 border-b border-destructive/40 bg-destructive/10 px-3 py-2"
    >
      <p class="text-xs">{{ describeShellError(translate, connectionFailure).message }}</p>
      <p class="mt-0.5 font-mono text-[11px] text-muted-foreground">
        {{ describeShellError(translate, connectionFailure).code }}
      </p>
    </div>

    <div
      v-if="connection === null || connection.phase === 'connecting'"
      class="flex flex-1 items-center justify-center gap-2"
    >
      <LoaderCircle :size="14" class="animate-spin text-muted-foreground" />
      <p class="text-xs text-muted-foreground">{{ t("k8s.connecting") }}</p>
    </div>

    <div v-else-if="connection.phase === 'error'" class="flex flex-1 flex-col items-center justify-center gap-2">
      <p class="text-xs text-muted-foreground">{{ t("k8s.notConnected") }}</p>
      <button
        type="button"
        class="flex items-center gap-1 rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent"
        @click="retry()"
      >
        <RefreshCw :size="12" />
        {{ t("k8s.retry") }}
      </button>
    </div>

    <div v-else class="flex min-h-0 flex-1">
      <aside class="w-44 shrink-0 overflow-hidden border-r border-border">
        <K8sTree :selected="selected" :connection-id="connId ?? undefined" class="h-full" @select="choose" />
      </aside>

      <div class="relative flex min-h-0 min-w-0 flex-1 flex-col">
        <K8sOverviewView
          v-if="selected === OVERVIEW_KEY"
          :overview="overview"
          :usage="clusterUsage"
          :metrics="metrics"
          :failure="overviewFailure"
          @navigate="(key: string) => (selected = key)"
        />

        <template v-else-if="descriptor">
          <div v-if="namespaced" class="flex shrink-0 items-center gap-2 border-b border-border px-3 py-1">
            <label class="text-[11px] text-muted-foreground" :for="namespaceListId">
              {{ t("k8s.list.namespace.label") }}
            </label>
            <input
              :id="namespaceListId"
              v-model="namespace"
              :list="`${namespaceListId}-options`"
              type="text"
              spellcheck="false"
              class="w-56 rounded-md border border-border bg-background px-2 py-0.5 font-mono text-[11px] focus:outline-none"
              :placeholder="t('k8s.list.namespace.all')"
              :aria-label="t('k8s.list.namespace.label')"
            />
            <datalist :id="`${namespaceListId}-options`">
              <option v-for="name in namespaceOptions" :key="name" :value="name" />
            </datalist>
          </div>

          <K8sResourceTable
            :key="descriptor.key"
            :descriptor="descriptor"
            :rows="rows"
            :loading="loading"
            :failure="listFailure"
            :watch-failure="watchFailure"
            :metrics-available="metrics.available"
            :usage-of="usageOf"
            :preset="presetFilter"
            @open="openDetail"
            @logs="(obj: K8sObject) => (logsTarget = obj)"
            @terminal="startTerminal"
            @action="onRowAction"
            @refresh="connId && store.refresh(connId, descriptor.key, namespace)"
            @create="openCreate"
          />
        </template>

        <K8sDetailDrawer
          :open="detailId !== null"
          :object="detailObject"
          :descriptor="descriptor"
          :events="events"
          :events-failure="eventsFailure"
          :events-loading="eventsLoading"
          @close="closeDetail"
        />

        <K8sLogsDrawer :open="logsTarget !== null" :conn-id="connId" :pod="logsTarget" @close="logsTarget = null" />

        <K8sActionPrompt
          :open="promptOpen"
          :kind="prompt?.kind ?? null"
          :descriptor="descriptor"
          :object="prompt?.object ?? null"
          :busy="promptBusy"
          @confirm="approve"
          @update:open="closePrompt"
        />

        <K8sExecDialog
          :open="execTarget !== null"
          :pod="execTarget"
          :busy="execBusy"
          :failure="execFailure"
          @confirm="(container: string) => execTarget && openShell(execTarget, container)"
          @update:open="closeExec"
        />

        <!--
          Which container a pod's files are browsed in. A picker of its own rather than the
          terminal's: the two open different things, and a dialog that said "terminal" while
          opening a browser would be lying about what the button does.
        -->
        <DialogRoot :open="filesTarget !== null" @update:open="onFilesOpenChange">
          <DialogPortal>
            <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
            <DialogContent
              class="fixed top-1/2 left-1/2 z-50 w-[min(26rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
            >
              <div class="px-4 pt-4">
                <DialogTitle class="text-sm font-medium">
                  {{ t("k8s.files.title", { pod: filesTarget ? nameOf(filesTarget) : "" }) }}
                </DialogTitle>
                <DialogDescription class="mt-1 text-xs text-muted-foreground">
                  {{ t("k8s.files.message") }}
                </DialogDescription>
              </div>

              <fieldset class="max-h-64 space-y-0.5 overflow-y-auto px-4 pt-3">
                <legend class="sr-only">{{ t("k8s.files.container") }}</legend>
                <label
                  v-for="name in filesContainers"
                  :key="name"
                  class="flex items-center gap-2 rounded-md px-1 py-1 font-mono text-[11px] transition-colors hover:bg-accent/50"
                >
                  <input
                    v-model="filesChosen"
                    type="radio"
                    :value="name"
                    name="k8s-files-container"
                    class="size-3 accent-current"
                  />
                  {{ name }}
                </label>
              </fieldset>

              <footer
                class="mt-3 flex flex-col-reverse gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3 sm:flex-row sm:justify-end"
              >
                <button
                  type="button"
                  class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
                  :disabled="filesBusy"
                  @click="closeFiles"
                >
                  {{ t("common.action.cancel") }}
                </button>
                <button
                  type="button"
                  class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
                  :disabled="filesBusy || filesChosen === ''"
                  @click="filesTarget && openFilesBrowser(filesTarget, filesChosen)"
                >
                  <LoaderCircle v-if="filesBusy" :size="12" class="mr-1.5 inline animate-spin" />
                  {{ t("k8s.files.open") }}
                </button>
              </footer>
            </DialogContent>
          </DialogPortal>
        </DialogRoot>

        <K8sCreateDialog
          :open="createOpen"
          :descriptor="descriptor"
          :namespace="namespace"
          :busy="creating"
          :failure="createFailure"
          @confirm="create"
          @update:open="(value: boolean) => (createOpen = value)"
        />
      </div>
    </div>
  </section>
</template>
