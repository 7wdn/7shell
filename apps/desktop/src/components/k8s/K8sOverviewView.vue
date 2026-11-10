<script setup lang="ts">
/**
 * What the cluster is, at a glance: how much of it is running and how hard it is working.
 *
 * Every number here comes from one pass of the store, so the tiles and the lists that open from them
 * cannot disagree. The usage meters read `allocatable` rather than capacity: a node keeps some of its
 * capacity for the kubelet and the system, and a percentage of the wrong denominator is a reading
 * somebody would act on.
 *
 * A cluster with no metrics server draws `—` in every usage slot and says so once. That is not an
 * error: the numbers do not exist anywhere in that cluster.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { formatCpu, formatMemory, percent } from "@/components/k8s/quantity";
import { RESOURCES } from "@/components/k8s/resources";
import type { ClusterOverview, MetricsState } from "@/stores/k8s";
import type { Usage } from "@/components/k8s/objects";
import { describeShellError } from "@/i18n/errors";
import type { ShellError } from "@/lib/backend/errors";
import { cn } from "@/lib/utils";

const props = defineProps<{
  overview: ClusterOverview | null;
  /** What the nodes are using, summed. Empty where the cluster has no metrics server. */
  usage: Usage;
  metrics: MetricsState;
  failure: ShellError | null;
}>();

const emit = defineEmits<{ navigate: [resourceKey: string] }>();

const { t } = useI18n();
const translate = (key: string, params?: Record<string, string>): string =>
  params === undefined ? t(key) : t(key, params);

const nodesReady = computed(() => (props.overview?.nodes ?? []).filter((node) => node.ready).length);
const nodesNotReady = computed(() => (props.overview?.nodes.length ?? 0) - nodesReady.value);

const allocatableCpu = computed(() =>
  (props.overview?.nodes ?? []).reduce((total, node) => total + node.allocatable.cpu, 0),
);
const allocatableMemory = computed(() =>
  (props.overview?.nodes ?? []).reduce((total, node) => total + node.allocatable.mem, 0),
);

const hasMetrics = computed(() => props.metrics.available && props.overview !== null);

/** Whether the meters have numbers to draw: both a used reading and something to divide by. */
const cpuKnown = computed(() => hasMetrics.value && allocatableCpu.value > 0);
const memoryKnown = computed(() => hasMetrics.value && allocatableMemory.value > 0);

function shareOf(used: number, total: number): number {
  return total > 0 ? Math.min(100, Math.round((used / total) * 100)) : 0;
}

const cpuPercent = computed(() => shareOf(props.usage.cpu, allocatableCpu.value));
const memoryPercent = computed(() => shareOf(props.usage.mem, allocatableMemory.value));
const cpuRequestPercent = computed(() => shareOf(props.overview?.requests.cpu ?? 0, allocatableCpu.value));
const memoryRequestPercent = computed(() => shareOf(props.overview?.requests.mem ?? 0, allocatableMemory.value));

/** Where a meter turns from a reading into a warning: past 90% there is nothing left to schedule. */
function meterTone(value: number): string {
  if (value >= 90) {
    return "bg-destructive";
  }
  if (value >= 70) {
    return "bg-warning";
  }
  return "bg-primary";
}

const phases = computed(() => {
  const counts = props.overview?.pods;
  return [
    { key: "running", label: "Running", count: counts?.running ?? 0 },
    { key: "pending", label: "Pending", count: counts?.pending ?? 0 },
    { key: "succeeded", label: "Succeeded", count: counts?.succeeded ?? 0 },
    { key: "failed", label: "Failed", count: counts?.failed ?? 0 },
    { key: "unknown", label: "Unknown", count: counts?.unknown ?? 0 },
  ];
});

/** The counts a group of the tree holds, so "Network" is one card rather than three tiles. */
function countsOfGroup(group: string): { key: string; label: string; count: number | null }[] {
  return RESOURCES.filter((desc) => desc.group === group).map((desc) => ({
    key: desc.key,
    label: t(desc.labelKey),
    count: props.overview?.counts[desc.key] ?? null,
  }));
}

const workloads = computed(() => countsOfGroup("workloads"));
const network = computed(() => countsOfGroup("network"));
const storage = computed(() => countsOfGroup("storage"));
</script>

<template>
  <div class="min-h-0 flex-1 overflow-auto p-3">
    <div
      v-if="props.failure"
      role="alert"
      class="mb-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2"
    >
      <p class="text-xs">{{ describeShellError(translate, props.failure).message }}</p>
      <p class="mt-0.5 font-mono text-[11px] text-muted-foreground">
        {{ describeShellError(translate, props.failure).code }}
      </p>
    </div>

    <div class="grid grid-cols-2 gap-2 lg:grid-cols-4">
      <button type="button" class="rounded-md border border-border bg-card px-3 py-2 text-left">
        <span class="block text-xs text-muted-foreground">{{ t("k8s.overview.version") }}</span>
        <span class="mt-0.5 block font-mono text-xs">{{ props.overview?.version || "—" }}</span>
      </button>

      <button
        type="button"
        class="rounded-md border border-border bg-card px-3 py-2 text-left transition-colors hover:bg-accent/50"
        @click="emit('navigate', 'nodes')"
      >
        <span class="block text-xs text-muted-foreground">{{ t("k8s.overview.nodes") }}</span>
        <span class="mt-0.5 block text-sm font-medium">{{ props.overview?.nodes.length ?? 0 }}</span>
        <span :class="cn('mt-0.5 block text-[11px]', nodesNotReady > 0 ? 'text-warning' : 'text-muted-foreground')">
          {{
            nodesNotReady > 0
              ? t("k8s.overview.notReady", { count: String(nodesNotReady) })
              : t("k8s.overview.allReady")
          }}
        </span>
      </button>

      <button
        type="button"
        class="rounded-md border border-border bg-card px-3 py-2 text-left transition-colors hover:bg-accent/50"
        @click="emit('navigate', 'pods')"
      >
        <span class="block text-xs text-muted-foreground">{{ t("k8s.resources.pods") }}</span>
        <span class="mt-0.5 block text-sm font-medium">{{ props.overview?.pods.total ?? 0 }}</span>
        <span class="mt-0.5 block text-[11px] text-muted-foreground">
          {{
            (props.overview?.pods.notReady ?? 0) > 0
              ? t("k8s.overview.notReady", { count: String(props.overview?.pods.notReady ?? 0) })
              : t("k8s.overview.allReady")
          }}
        </span>
      </button>

      <button
        type="button"
        class="rounded-md border border-border bg-card px-3 py-2 text-left transition-colors hover:bg-accent/50"
        @click="emit('navigate', 'namespaces')"
      >
        <span class="block text-xs text-muted-foreground">{{ t("k8s.overview.namespaces") }}</span>
        <span class="mt-0.5 block text-sm font-medium">{{ props.overview?.counts.namespaces ?? "—" }}</span>
      </button>
    </div>

    <p v-if="!props.metrics.available" class="mt-3 text-xs text-muted-foreground">
      {{ t("k8s.overview.noMetrics") }}
    </p>
    <p v-else-if="props.metrics.failure" class="mt-3 text-xs text-destructive">
      {{ describeShellError(translate, props.metrics.failure).message }}
    </p>

    <div class="mt-3 grid gap-2 lg:grid-cols-2">
      <section class="rounded-md border border-border bg-card px-3 py-2">
        <h3 class="text-xs font-medium">{{ t("k8s.overview.cpu") }}</h3>
        <div class="mt-2 space-y-2">
          <div>
            <div class="flex items-baseline justify-between gap-2 text-[11px]">
              <span class="text-muted-foreground">{{ t("k8s.overview.usage") }}</span>
              <span class="font-mono">
                {{ cpuKnown ? `${formatCpu(props.usage.cpu)} / ${formatCpu(allocatableCpu)}` : "—" }}
              </span>
              <span class="w-10 text-right font-mono text-muted-foreground">
                {{ cpuKnown ? percent(props.usage.cpu, allocatableCpu) : "—" }}
              </span>
            </div>
            <div class="mt-1 h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                v-if="cpuKnown"
                :class="cn('h-full rounded-full', meterTone(cpuPercent))"
                :style="{ width: `${cpuPercent}%` }"
              />
            </div>
          </div>
          <div>
            <div class="flex items-baseline justify-between gap-2 text-[11px]">
              <span class="text-muted-foreground">{{ t("k8s.overview.requested") }}</span>
              <span class="font-mono">
                {{
                  allocatableCpu > 0
                    ? `${formatCpu(props.overview?.requests.cpu ?? 0)} / ${formatCpu(allocatableCpu)}`
                    : "—"
                }}
              </span>
              <span class="w-10 text-right font-mono text-muted-foreground">
                {{ allocatableCpu > 0 ? percent(props.overview?.requests.cpu ?? 0, allocatableCpu) : "—" }}
              </span>
            </div>
            <div class="mt-1 h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                v-if="allocatableCpu > 0"
                :class="cn('h-full rounded-full', meterTone(cpuRequestPercent))"
                :style="{ width: `${cpuRequestPercent}%` }"
              />
            </div>
          </div>
        </div>
      </section>

      <section class="rounded-md border border-border bg-card px-3 py-2">
        <h3 class="text-xs font-medium">{{ t("k8s.overview.memory") }}</h3>
        <div class="mt-2 space-y-2">
          <div>
            <div class="flex items-baseline justify-between gap-2 text-[11px]">
              <span class="text-muted-foreground">{{ t("k8s.overview.usage") }}</span>
              <span class="font-mono">
                {{ memoryKnown ? `${formatMemory(props.usage.mem)} / ${formatMemory(allocatableMemory)}` : "—" }}
              </span>
              <span class="w-10 text-right font-mono text-muted-foreground">
                {{ memoryKnown ? percent(props.usage.mem, allocatableMemory) : "—" }}
              </span>
            </div>
            <div class="mt-1 h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                v-if="memoryKnown"
                :class="cn('h-full rounded-full', meterTone(memoryPercent))"
                :style="{ width: `${memoryPercent}%` }"
              />
            </div>
          </div>
          <div>
            <div class="flex items-baseline justify-between gap-2 text-[11px]">
              <span class="text-muted-foreground">{{ t("k8s.overview.requested") }}</span>
              <span class="font-mono">
                {{
                  allocatableMemory > 0
                    ? `${formatMemory(props.overview?.requests.mem ?? 0)} / ${formatMemory(allocatableMemory)}`
                    : "—"
                }}
              </span>
              <span class="w-10 text-right font-mono text-muted-foreground">
                {{ allocatableMemory > 0 ? percent(props.overview?.requests.mem ?? 0, allocatableMemory) : "—" }}
              </span>
            </div>
            <div class="mt-1 h-1.5 overflow-hidden rounded-full bg-muted">
              <div
                v-if="allocatableMemory > 0"
                :class="cn('h-full rounded-full', meterTone(memoryRequestPercent))"
                :style="{ width: `${memoryRequestPercent}%` }"
              />
            </div>
          </div>
        </div>
      </section>
    </div>

    <div class="mt-3 grid gap-2 lg:grid-cols-4">
      <section class="rounded-md border border-border bg-card px-3 py-2">
        <h3 class="text-xs font-medium">{{ t("k8s.overview.podPhases") }}</h3>
        <button
          v-for="phase in phases"
          :key="phase.key"
          type="button"
          class="mt-1 flex w-full items-center justify-between gap-2 rounded-sm px-1 py-0.5 text-xs transition-colors hover:bg-accent/50"
          @click="emit('navigate', 'pods')"
        >
          <span class="text-muted-foreground">{{ phase.label }}</span>
          <span class="font-mono">{{ phase.count }}</span>
        </button>
      </section>

      <section
        v-for="card in [
          { key: 'workloads', label: t('k8s.groups.workloads'), rows: workloads },
          { key: 'network', label: t('k8s.groups.network'), rows: network },
          { key: 'storage', label: t('k8s.groups.storage'), rows: storage },
        ]"
        :key="card.key"
        class="rounded-md border border-border bg-card px-3 py-2"
      >
        <h3 class="text-xs font-medium">{{ card.label }}</h3>
        <button
          v-for="row in card.rows"
          :key="row.key"
          type="button"
          class="mt-1 flex w-full items-center justify-between gap-2 rounded-sm px-1 py-0.5 text-xs transition-colors hover:bg-accent/50"
          @click="emit('navigate', row.key)"
        >
          <span class="truncate text-muted-foreground">{{ row.label }}</span>
          <span class="font-mono">{{ row.count ?? "—" }}</span>
        </button>
      </section>
    </div>
  </div>
</template>
