<script setup lang="ts">
/**
 * One collection, as a table: a filter, an order, and the actions a row offers.
 *
 * The columns come from the descriptor, so this component knows nothing about pods or ingresses —
 * it knows that a column has text and maybe a number to sort by. The one thing it does know is which
 * icon an action wears, because an action is a control rather than data.
 *
 * Sorting falls back to reading a number out of the text (a percentage, an age, a `2/3`), which is
 * what makes pressing "Age" order by time rather than alphabetically. A column that carries its own
 * amount — CPU's millicores, a restart count — hands that over instead.
 */
import {
  ArrowUpDown,
  Ban,
  Box,
  CircleCheck,
  CirclePower,
  LoaderCircle,
  Plus,
  RefreshCw,
  Repeat,
  ScrollText,
  Search,
  SquareTerminal,
  Trash2,
} from "@lucide/vue";
import { type Component, computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import SortHeader from "@/components/ui/SortHeader.vue";
import { boolAt, nameOf, namespaceOf, type K8sObject, type Usage } from "@/components/k8s/objects";
import { numericValueOf } from "@/components/k8s/quantity";
import type { ColumnDef, ResourceAction, ResourceDescriptor } from "@/components/k8s/resources";
import { describeShellError } from "@/i18n/errors";
import type { ShellError } from "@/lib/backend/errors";
import { TABLE, rowClass } from "@/lib/table";
import { cn } from "@/lib/utils";

const props = defineProps<{
  descriptor: ResourceDescriptor;
  rows: readonly K8sObject[];
  loading: boolean;
  failure: ShellError | null;
  /** Why the list is not following the cluster. The rows are still true; this is a hint. */
  watchFailure: ShellError | null;
  /** False where the cluster has no metrics API, which turns every usage cell into `—`. */
  metricsAvailable: boolean;
  /** What one row is using, when the kind carries metrics. */
  usageOf: (obj: K8sObject) => Usage | null;
  /** A text the list starts filtered by, when another row sent somebody here (a workload's pods). */
  preset?: string;
}>();

const emit = defineEmits<{
  open: [obj: K8sObject];
  logs: [obj: K8sObject];
  terminal: [obj: K8sObject];
  action: [obj: K8sObject, action: ResourceAction];
  refresh: [];
  create: [];
}>();

const { t } = useI18n();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const ICONS: Readonly<Record<ResourceAction, Component>> = {
  logs: ScrollText,
  terminal: SquareTerminal,
  viewPods: Box,
  restart: Repeat,
  scale: ArrowUpDown,
  cordon: Ban,
  drain: CirclePower,
  delete: Trash2,
};

/** The label a row's action wears, which for cordon depends on which way the node is now. */
function actionLabel(action: ResourceAction, obj: K8sObject): string {
  if (action === "cordon") {
    return boolAt(obj, "spec", "unschedulable") ? t("k8s.actions.uncordon") : t("k8s.actions.cordon");
  }
  switch (action) {
    case "logs":
      return t("k8s.actions.logs");
    case "terminal":
      return t("k8s.actions.terminal");
    case "viewPods":
      return t("k8s.actions.viewPods");
    case "restart":
      return t("k8s.actions.restart");
    case "scale":
      return t("k8s.actions.scale");
    case "drain":
      return t("k8s.actions.drain");
    default:
      return t("k8s.actions.delete");
  }
}

function actionIcon(action: ResourceAction, obj: K8sObject): Component {
  if (action === "cordon") {
    return boolAt(obj, "spec", "unschedulable") ? CircleCheck : Ban;
  }
  return ICONS[action];
}

const filter = ref("");
const order = ref<{ readonly key: string; readonly descending: boolean }>({ key: "name", descending: false });

/**
 * A new kind starts from its name order, and from the filter it was sent here with.
 *
 * Carrying the last kind's filter over would be a lie about what this list holds, and so would
 * keeping a filter that belonged to a row somebody has navigated away from.
 */
watch(
  () => [props.descriptor.key, props.preset] as const,
  () => {
    filter.value = props.preset ?? "";
    order.value = { key: "name", descending: false };
  },
  { immediate: true },
);

function cellText(column: ColumnDef, obj: K8sObject): string {
  return column.text(obj, props.usageOf(obj));
}

function visible(obj: K8sObject): boolean {
  const wanted = filter.value.trim().toLowerCase();
  if (wanted === "") {
    return true;
  }
  if (nameOf(obj).toLowerCase().includes(wanted)) {
    return true;
  }
  return props.descriptor.columns.some(
    (column) => column.searchable === true && cellText(column, obj).toLowerCase().includes(wanted),
  );
}

function compare(left: K8sObject, right: K8sObject): number {
  const column = props.descriptor.columns.find((entry) => entry.key === order.value.key);
  if (!column) {
    return nameOf(left).localeCompare(nameOf(right));
  }
  const direction = order.value.descending ? -1 : 1;
  if (column.sortKey) {
    return (column.sortKey(left, props.usageOf(left)) - column.sortKey(right, props.usageOf(right))) * direction;
  }
  const leftText = cellText(column, left);
  const rightText = cellText(column, right);
  const leftNumber = numericValueOf(leftText);
  const rightNumber = numericValueOf(rightText);
  if (leftNumber !== null && rightNumber !== null) {
    return (leftNumber - rightNumber) * direction;
  }
  // A reading that is not an amount sorts after one that is: `—` belongs at the bottom, not the top.
  if (leftNumber !== null) {
    return -1 * direction;
  }
  if (rightNumber !== null) {
    return 1 * direction;
  }
  return leftText.localeCompare(rightText) * direction;
}

/**
 * The rows, filtered and ordered.
 *
 * The name is the final tie-break so that two pods with equal readings keep a stable order: without
 * it, clicking a heading twice can shuffle rows that compare equal, which reads as a flicker.
 */
const visibleRows = computed(() => {
  const filtered = props.rows.filter(visible);
  return [...filtered].sort((left, right) => compare(left, right) || nameOf(left).localeCompare(nameOf(right)));
});

function orderBy(column: ColumnDef): void {
  order.value =
    order.value.key === column.key
      ? { key: column.key, descending: !order.value.descending }
      : { key: column.key, descending: false };
}

/** The row's health, when the kind has one: a broken pod should be findable without reading every cell. */
function rowTone(obj: K8sObject): string {
  const tone = props.descriptor.tone?.(obj) ?? "";
  if (tone === "err") {
    return "bg-destructive/5";
  }
  return tone === "warn" ? "bg-warning/5" : "";
}

const columnCount = computed(() => props.descriptor.columns.length + (props.descriptor.actions.length > 0 ? 1 : 0));

const showCreate = computed(() => props.descriptor.createTemplate !== undefined);

/** A row's identity, which an all-namespaces list needs: two namespaces may hold the same name. */
function keyOf(obj: K8sObject): string {
  return `${namespaceOf(obj)}/${nameOf(obj)}`;
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col">
    <div class="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-3 py-1.5">
      <label class="relative flex min-w-40 flex-1 items-center">
        <Search :size="12" class="pointer-events-none absolute left-2 text-muted-foreground" />
        <input
          v-model="filter"
          type="search"
          spellcheck="false"
          class="w-full rounded-md border border-border bg-background py-1 pr-2 pl-7 text-xs focus:outline-none"
          :placeholder="t('k8s.list.filter')"
          :aria-label="t('k8s.list.filter')"
        />
      </label>

      <span class="font-mono text-[11px] text-muted-foreground">
        {{ t("k8s.list.count", { count: String(visibleRows.length) }) }}
      </span>

      <button
        type="button"
        class="grid size-6 place-items-center rounded-md border border-border text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('k8s.list.refresh')"
        :aria-label="t('k8s.list.refresh')"
        @click="emit('refresh')"
      >
        <LoaderCircle v-if="props.loading" :size="12" class="animate-spin" />
        <RefreshCw v-else :size="12" />
      </button>

      <button
        v-if="showCreate"
        type="button"
        class="flex items-center gap-1 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
        @click="emit('create')"
      >
        <Plus :size="12" />
        {{ t("k8s.list.create") }}
      </button>
    </div>

    <p v-if="props.failure" role="alert" class="shrink-0 border-b border-destructive/40 bg-destructive/10 px-3 py-2">
      <span class="block text-xs">{{ describeShellError(translate, props.failure).message }}</span>
      <span class="mt-0.5 block font-mono text-[11px] text-muted-foreground">
        {{ describeShellError(translate, props.failure).code }}
      </span>
    </p>

    <p v-else-if="props.watchFailure" class="shrink-0 border-b border-warning/40 bg-warning/10 px-3 py-1.5 text-[11px]">
      {{ t("k8s.list.notLive", { reason: describeShellError(translate, props.watchFailure).message }) }}
    </p>

    <p
      v-else-if="!props.metricsAvailable"
      class="shrink-0 border-b border-border px-3 py-1.5 text-[11px] text-muted-foreground"
    >
      {{ t("k8s.list.metricsMissing") }}
    </p>

    <div class="min-h-0 flex-1 overflow-auto">
      <table
        :class="cn(TABLE.page.table, 'w-full')"
        :aria-label="t('k8s.list.table', { kind: t(props.descriptor.labelKey) })"
      >
        <thead :class="TABLE.page.head">
          <tr :class="TABLE.page.headRow">
            <th
              v-for="column in props.descriptor.columns"
              :key="column.key"
              scope="col"
              :class="cn(TABLE.page.headCell, 'text-left whitespace-nowrap')"
              :aria-sort="order.key === column.key ? (order.descending ? 'descending' : 'ascending') : 'none'"
            >
              <SortHeader
                :label="t(column.headerKey)"
                :sorted="order.key === column.key"
                :descending="order.descending"
                :hint="t('k8s.list.sortBy', { column: t(column.headerKey) })"
                @sort="orderBy(column)"
              />
            </th>
            <th
              v-if="props.descriptor.actions.length > 0"
              scope="col"
              :class="cn(TABLE.page.headCell, 'w-px text-right whitespace-nowrap')"
            >
              {{ t("k8s.actions.title") }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="props.loading && visibleRows.length === 0">
            <td :class="TABLE.page.emptyCell" :colspan="columnCount">
              <LoaderCircle :size="12" class="mr-1.5 inline animate-spin" />
              {{ t("k8s.list.loading") }}
            </td>
          </tr>
          <tr v-else-if="visibleRows.length === 0">
            <td :class="TABLE.page.emptyCell" :colspan="columnCount">
              {{ filter.trim() === "" ? t("k8s.list.empty") : t("k8s.list.noMatch") }}
            </td>
          </tr>
          <tr
            v-for="obj in visibleRows"
            :key="keyOf(obj)"
            :class="cn(rowClass('page', { clickable: true }), rowTone(obj))"
            @click="emit('open', obj)"
          >
            <td
              v-for="column in props.descriptor.columns"
              :key="column.key"
              :class="cn(TABLE.page.cell, 'max-w-64 truncate whitespace-nowrap', column.key === 'name' && 'font-mono')"
              :title="cellText(column, obj)"
            >
              {{ cellText(column, obj) }}
            </td>
            <td v-if="props.descriptor.actions.length > 0" :class="cn(TABLE.page.cell, 'w-px whitespace-nowrap')">
              <div class="flex items-center justify-end gap-0.5">
                <button
                  v-for="action in props.descriptor.actions"
                  :key="action"
                  type="button"
                  class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  :class="action === 'delete' && 'hover:text-destructive'"
                  :title="actionLabel(action, obj)"
                  :aria-label="actionLabel(action, obj)"
                  @click.stop="
                    action === 'logs'
                      ? emit('logs', obj)
                      : action === 'terminal'
                        ? emit('terminal', obj)
                        : emit('action', obj, action)
                  "
                >
                  <component :is="actionIcon(action, obj)" :size="12" />
                </button>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
    </div>
  </div>
</template>
