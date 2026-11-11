<script setup lang="ts">
/**
 * The kinds of thing a cluster holds, grouped the way the API's own documentation groups them.
 *
 * It is driven by the descriptor table rather than by a list of buttons here: a kind that exists in
 * the table is drawn, and a kind added there needs no change in this file. The groups are collapsible
 * because a cluster has more kinds than a sidebar has room for, and Access is closed by default —
 * it is the group somebody opens on purpose. The last group is not in the table: it is whatever
 * kinds the cluster's CRDs define, which is why those entries carry the cluster's own names.
 */
import {
  Bell,
  Box,
  Cable,
  ChevronDown,
  ChevronRight,
  Clock,
  Copy,
  Database,
  FileText,
  Folder,
  Gauge,
  Globe,
  HardDrive,
  Layers,
  Lock,
  Network,
  Package,
  Server,
  Shield,
} from "@lucide/vue";
import { type Component, computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { OVERVIEW_KEY, resourceGroups, type ResourceIcon } from "@/components/k8s/resources";
import { cn } from "@/lib/utils";

const props = defineProps<{ selected: string; connectionId?: string }>();
const emit = defineEmits<{ select: [key: string] }>();

const { t } = useI18n();

/** Which icon stands for which kind. A table, so a descriptor names an icon rather than importing one. */
const ICONS: Readonly<Record<ResourceIcon, Component>> = {
  box: Box,
  layers: Layers,
  database: Database,
  copy: Copy,
  clock: Clock,
  network: Network,
  globe: Globe,
  file: FileText,
  lock: Lock,
  drive: HardDrive,
  server: Server,
  folder: Folder,
  bell: Bell,
  shield: Shield,
  cable: Cable,
  package: Package,
  gauge: Gauge,
};

const groups = computed(() => resourceGroups(props.connectionId));

/**
 * Which groups are open.
 *
 * Access is shut to begin with: roles and bindings are read when something is being granted, not
 * while somebody is looking for a pod, and an open group nobody uses is a row in the way.
 */
const closed = ref<Set<string>>(new Set(["rbac"]));

function toggle(group: string): void {
  const next = new Set(closed.value);
  if (next.has(group)) {
    next.delete(group);
  } else {
    next.add(group);
  }
  closed.value = next;
}
</script>

<template>
  <nav class="flex h-full min-h-0 flex-col overflow-y-auto py-1" :aria-label="t('k8s.tree')">
    <button
      type="button"
      :class="
        cn(
          'flex w-full items-center gap-1.5 px-2 py-1 text-left text-xs transition-colors',
          selected === OVERVIEW_KEY ? 'bg-accent/60 text-foreground' : 'text-muted-foreground hover:bg-accent/50',
        )
      "
      :aria-current="selected === OVERVIEW_KEY ? 'true' : undefined"
      @click="emit('select', OVERVIEW_KEY)"
    >
      <span class="size-3 shrink-0" />
      <Gauge :size="13" class="shrink-0 text-muted-foreground" />
      <span class="truncate">{{ t("k8s.overview.label") }}</span>
    </button>

    <div v-for="entry in groups" :key="entry.group" class="mt-1">
      <button
        type="button"
        class="flex w-full items-center gap-1 px-2 py-1 text-left text-xs font-medium text-muted-foreground transition-colors hover:bg-accent/40"
        :aria-expanded="!closed.has(entry.group)"
        @click="toggle(entry.group)"
      >
        <ChevronRight v-if="closed.has(entry.group)" :size="12" class="shrink-0" />
        <ChevronDown v-else :size="12" class="shrink-0" />
        <span class="truncate">{{ t(`k8s.groups.${entry.group}`) }}</span>
      </button>

      <template v-if="!closed.has(entry.group)">
        <button
          v-for="resource in entry.resources"
          :key="resource.key"
          type="button"
          :class="
            cn(
              'flex w-full items-center gap-1.5 py-1 pr-2 pl-6 text-left text-xs transition-colors',
              selected === resource.key ? 'bg-accent/60 text-foreground' : 'text-muted-foreground hover:bg-accent/50',
            )
          "
          :aria-current="selected === resource.key ? 'true' : undefined"
          :title="resource.kindLabel ? `${resource.kindLabel} (${resource.apiVersion})` : undefined"
          @click="emit('select', resource.key)"
        >
          <component :is="ICONS[resource.icon]" :size="13" class="shrink-0 opacity-80" />
          <span class="truncate">{{ resource.kindLabel ?? t(resource.labelKey) }}</span>
        </button>
      </template>
    </div>
  </nav>
</template>
