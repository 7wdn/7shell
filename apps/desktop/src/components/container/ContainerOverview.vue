<script setup lang="ts">
/**
 * What the engine is, in one screen: the runtime, the host behind it, and what it holds.
 *
 * The counts are the part that is worth reaching for: a state count is also a filter, so clicking
 * "exited 12" is how somebody gets from "twelve of these are not running" to the twelve rows. That
 * is why this view emits rather than owning the filter — the list is where it belongs.
 *
 * A runtime that cannot describe itself leaves the cards empty rather than failing the page: `wslc`
 * answers fewer questions than `docker`, and an empty field is an honest answer.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { formatSize } from "@/lib/files/format";
import type { ContainerSession } from "@/stores/container";
import type { ContainerTab } from "@/stores/terminals";

const props = defineProps<{ tab: ContainerTab; session: ContainerSession }>();
const emit = defineEmits<{ "show-state": [state: string]; "show-images": [] }>();

const { t } = useI18n();

function withComponent(version: string | undefined, component: string | undefined): string {
  if (!version) {
    return "";
  }
  return component ? `${version} (${component})` : version;
}

const runtimeFields = computed<[string, string][]>(() => {
  const info = props.session.info;
  const cgroup = [info?.cgroupDriver, info?.cgroupVersion].filter(Boolean).join(" ");
  return [
    [t("container.overview.runtime"), props.session.runtime],
    [t("container.overview.client"), withComponent(info?.clientVersion, info?.clientComponent)],
    [t("container.overview.server"), withComponent(info?.serverVersion, info?.serverComponent)],
    [t("container.overview.storageDriver"), info?.storageDriver ?? ""],
    [t("container.overview.cgroup"), cgroup],
  ];
});

const hostFields = computed<[string, string][]>(() => {
  const info = props.session.info;
  return [
    [t("container.overview.os"), info?.os ?? ""],
    [t("container.overview.arch"), info?.arch ?? ""],
    [t("container.overview.kernel"), info?.kernel ?? ""],
    [t("container.overview.cpus"), info && info.cpus > 0 ? String(info.cpus) : ""],
    [t("container.overview.memory"), info && info.memory > 0 ? formatSize(info.memory) : ""],
  ];
});

/**
 * The states the engine reported, busiest first.
 *
 * Taken from the info call when it answered and from the list otherwise: the list is what the table
 * will show, and a count that disagrees with the rows under it is worse than no count at all.
 */
const stateCounts = computed(() => {
  const fromInfo = props.session.info?.containersByState ?? [];
  if (fromInfo.length > 0) {
    return [...fromInfo].sort((left, right) => right.count - left.count);
  }
  const counts = new Map<string, number>();
  for (const container of props.session.containers) {
    counts.set(container.state, (counts.get(container.state) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([state, count]) => ({ state, count }))
    .sort((left, right) => right.count - left.count);
});

const containersTotal = computed(() => props.session.info?.containersTotal ?? props.session.containers.length);
const imagesTotal = computed(() => props.session.info?.imagesTotal ?? props.session.images.length);
const imagesTagged = computed(() => props.session.info?.imagesTagged ?? 0);
const imagesDangling = computed(() => props.session.info?.imagesDangling ?? 0);

/** States that have a colour of their own; anything else is drawn neutral. */
function stateDotClass(state: string): string {
  switch (state) {
    case "running":
      return "bg-success";
    case "paused":
      return "bg-warning";
    case "dead":
      return "bg-destructive";
    default:
      return "bg-muted-foreground/50";
  }
}
</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col overflow-y-auto p-3">
    <div class="grid grid-cols-[repeat(auto-fit,minmax(15rem,1fr))] items-start gap-3">
      <section class="rounded-md border border-border bg-card px-3 py-2.5">
        <h3 class="mb-2 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("container.overview.runtime") }}
        </h3>
        <dl class="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-x-4 gap-y-2">
          <div v-for="field in runtimeFields" :key="field[0]" class="flex min-w-0 flex-col gap-0.5">
            <dt class="text-[10px] text-muted-foreground">{{ field[0] }}</dt>
            <dd class="font-mono text-[11px] break-all select-text">{{ field[1] || "—" }}</dd>
          </div>
        </dl>
      </section>

      <section class="rounded-md border border-border bg-card px-3 py-2.5">
        <h3 class="mb-2 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("container.overview.host") }}
        </h3>
        <dl class="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-x-4 gap-y-2">
          <div v-for="field in hostFields" :key="field[0]" class="flex min-w-0 flex-col gap-0.5">
            <dt class="text-[10px] text-muted-foreground">{{ field[0] }}</dt>
            <dd class="font-mono text-[11px] break-all select-text">{{ field[1] || "—" }}</dd>
          </div>
        </dl>
      </section>
    </div>

    <div class="mt-3 grid grid-cols-[repeat(auto-fit,minmax(18rem,1fr))] items-start gap-3">
      <section class="rounded-md border border-border bg-card px-3 py-2.5">
        <h3 class="mb-1.5 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("container.overview.containers") }}
        </h3>
        <button
          type="button"
          class="flex w-full items-center justify-between gap-3 rounded-sm px-1.5 py-1 text-left transition-colors hover:bg-accent/50"
          @click="emit('show-state', '')"
        >
          <span class="text-[11px] text-muted-foreground">{{ t("container.overview.total") }}</span>
          <span class="font-mono text-[11px] font-medium">{{ containersTotal }}</span>
        </button>
        <button
          v-for="entry in stateCounts"
          :key="entry.state"
          type="button"
          class="flex w-full items-center justify-between gap-3 rounded-sm px-1.5 py-1 text-left transition-colors hover:bg-accent/50"
          @click="emit('show-state', entry.state)"
        >
          <span class="flex min-w-0 items-center gap-1.5 text-[11px] text-muted-foreground">
            <span class="size-1.5 shrink-0 rounded-full" :class="stateDotClass(entry.state)"></span>
            <span class="truncate">{{ entry.state }}</span>
          </span>
          <span class="font-mono text-[11px]">{{ entry.count }}</span>
        </button>
        <p v-if="stateCounts.length === 0" class="px-1.5 py-1 text-[11px] text-muted-foreground">—</p>
      </section>

      <section class="rounded-md border border-border bg-card px-3 py-2.5">
        <h3 class="mb-1.5 text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("container.overview.images") }}
        </h3>
        <button
          type="button"
          class="flex w-full items-center justify-between gap-3 rounded-sm px-1.5 py-1 text-left transition-colors hover:bg-accent/50"
          @click="emit('show-images')"
        >
          <span class="text-[11px] text-muted-foreground">{{ t("container.overview.total") }}</span>
          <span class="font-mono text-[11px] font-medium">{{ imagesTotal }}</span>
        </button>
        <button
          type="button"
          class="flex w-full items-center justify-between gap-3 rounded-sm px-1.5 py-1 text-left transition-colors hover:bg-accent/50"
          @click="emit('show-images')"
        >
          <span class="text-[11px] text-muted-foreground">{{ t("container.overview.tagged") }}</span>
          <span class="font-mono text-[11px]">{{ imagesTagged }}</span>
        </button>
        <button
          type="button"
          class="flex w-full items-center justify-between gap-3 rounded-sm px-1.5 py-1 text-left transition-colors hover:bg-accent/50"
          @click="emit('show-images')"
        >
          <span class="text-[11px] text-muted-foreground">{{ t("container.overview.dangling") }}</span>
          <span class="font-mono text-[11px]">{{ imagesDangling }}</span>
        </button>
      </section>
    </div>
  </div>
</template>
