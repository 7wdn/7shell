<script setup lang="ts">
/**
 * The container workspace: one engine, seen three ways.
 *
 * All three views stay mounted and are hidden with `v-show`, for the same reason the pane itself is
 * kept mounted across tabs: a log stream, a pull in progress or half-typed filter must survive a look
 * at another view. Unmounting on a switch would stop the stream, and the line somebody was waiting
 * for would have gone by unseen.
 *
 * The connection is opened here once, on mount, and closed by `terminals` when the tab goes away. On
 * unmount this pane only forgets its share of the state — see `useContainerStore.forget`.
 */
import { Boxes, Gauge, Layers, LoaderCircle, RotateCw, X } from "@lucide/vue";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import Select from "@/components/ui/Select.vue";
import { describeShellError } from "@/i18n/errors";
import { cn } from "@/lib/utils";
import { supportsNamespace, useContainerStore } from "@/stores/container";
import type { ContainerTab } from "@/stores/terminals";

import ContainerImageView from "./ContainerImageView.vue";
import ContainerListView from "./ContainerListView.vue";
import ContainerOverview from "./ContainerOverview.vue";

const props = defineProps<{ tab: ContainerTab; active: boolean }>();

type ContainerView = "overview" | "containers" | "images";

const { t } = useI18n();
const containers = useContainerStore();

const view = ref<ContainerView>("overview");
/** Set when the images view asked to see which containers run an image. */
const imageFilter = ref<string | null>(null);
/** Set when the overview's state counts were clicked. */
const stateFilter = ref<string | null>(null);

const session = computed(() => containers.sessionFor(props.tab.id));
const runtime = computed(() => props.tab.session.container?.runtime ?? "docker");
const problem = computed(() => (session.value?.error ? describeShellError(translate, session.value.error) : null));

const rail = computed(() => [
  {
    view: "overview" as const,
    labelKey: "container.rail.overview",
    icon: Gauge,
    count: null as number | null,
  },
  {
    view: "containers" as const,
    labelKey: "container.rail.containers",
    icon: Boxes,
    count: session.value?.containers.length ?? null,
  },
  {
    view: "images" as const,
    labelKey: "container.rail.images",
    icon: Layers,
    count: session.value?.images.length ?? null,
  },
]);

/** The namespace picker's choices: what the runtime listed, plus the default and the chosen one. */
const namespaceOptions = computed(() => {
  const current = session.value;
  if (!current) {
    return [];
  }
  const names = new Set(current.namespaces);
  names.add(current.namespace);
  names.add("default");
  return [...names].sort().map((name) => ({ value: name, label: name }));
});

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function dismiss(): void {
  containers.dismissError(props.tab.id);
}

function retry(): void {
  void containers.connect(props.tab);
}

function showState(state: string): void {
  imageFilter.value = null;
  // The total is the absence of a state filter, not a state called "": the list reads `null` as
  // "any state" and an empty string as a state nothing matches.
  stateFilter.value = state === "" ? null : state;
  view.value = "containers";
}

function showRelatedContainers(reference: string): void {
  stateFilter.value = null;
  imageFilter.value = reference;
  view.value = "containers";
}

function chooseNamespace(namespace: string): void {
  if (namespace === session.value?.namespace) {
    return;
  }
  void containers.setNamespace(props.tab.id, namespace);
}

onMounted(() => {
  void containers.connect(props.tab);
});

onBeforeUnmount(() => {
  // Not `close`: the tab's own close path has already disconnected by `tab.connectionId`.
  containers.forget(props.tab.id);
});
</script>

<template>
  <section class="relative flex min-h-0 flex-1 flex-col">
    <header class="flex shrink-0 items-center gap-2 border-b border-border px-3 py-[var(--panel-header-y)]">
      <Boxes :size="13" class="shrink-0 text-muted-foreground" />
      <h2 class="min-w-0 truncate text-xs font-medium" :title="props.tab.session.name">
        {{ props.tab.session.name }}
      </h2>
      <span class="shrink-0 rounded-sm border border-border px-1 font-mono text-[10px] text-muted-foreground">
        {{ runtime }}
      </span>

      <LoaderCircle
        v-if="session?.loading || session?.refreshing"
        :size="12"
        class="shrink-0 animate-spin text-muted-foreground"
        :title="t('container.loading')"
      />

      <div class="min-w-0 flex-1"></div>

      <template v-if="supportsNamespace(runtime) && session?.connectionId">
        <span class="shrink-0 text-[10px] text-muted-foreground">{{ t("container.namespace") }}</span>
        <Select
          :model-value="session.namespace"
          :options="namespaceOptions"
          size="compact"
          class="w-32"
          :aria-label="t('container.namespace')"
          @update:model-value="chooseNamespace"
        />
      </template>
    </header>

    <!-- The connection as it is being made, and as it failed to be made. -->
    <div
      v-if="!session || (session.loading && session.connectionId === null)"
      class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center"
    >
      <LoaderCircle :size="16" class="animate-spin text-muted-foreground" />
      <p class="text-xs text-muted-foreground">{{ t("container.connecting") }}</p>
    </div>

    <div
      v-else-if="session.connectionId === null"
      class="flex min-h-0 flex-1 flex-col items-center justify-center gap-2 p-6 text-center"
    >
      <p class="text-xs text-muted-foreground">{{ t("container.connectFailed") }}</p>
      <p v-if="problem" class="max-w-xl text-xs text-destructive select-text">{{ problem.message }}</p>
      <p v-if="problem" class="font-mono text-[10px] text-muted-foreground">{{ problem.code }}</p>
      <button
        type="button"
        class="mt-1 flex items-center gap-1.5 rounded-md border border-border px-2 py-1 text-xs transition-colors hover:bg-accent"
        @click="retry()"
      >
        <RotateCw :size="12" />
        {{ t("container.retry") }}
      </button>
    </div>

    <div v-else class="flex min-h-0 flex-1">
      <nav
        class="flex w-36 shrink-0 flex-col gap-0.5 overflow-y-auto border-r border-border p-1.5"
        :aria-label="t('container.rail.label')"
      >
        <button
          v-for="item in rail"
          :key="item.view"
          type="button"
          :class="
            cn(
              'flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-xs transition-colors',
              view === item.view
                ? 'bg-accent text-foreground'
                : 'text-muted-foreground hover:bg-accent/50 hover:text-foreground',
            )
          "
          :aria-current="view === item.view ? 'page' : undefined"
          @click="view = item.view"
        >
          <component :is="item.icon" :size="13" class="shrink-0" />
          <span class="min-w-0 flex-1 truncate text-left">{{ t(item.labelKey) }}</span>
          <span v-if="item.count !== null" class="shrink-0 font-mono text-[10px] text-muted-foreground">
            {{ item.count }}
          </span>
        </button>
      </nav>

      <div class="relative flex min-h-0 min-w-0 flex-1 flex-col">
        <div
          v-if="problem"
          role="alert"
          class="flex shrink-0 items-start gap-2 border-b border-destructive/40 bg-destructive/10 px-3 py-1.5 select-text"
        >
          <span class="min-w-0 flex-1">
            <span class="block text-[11px]">{{ problem.message }}</span>
            <span class="mt-0.5 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
          </span>
          <button
            type="button"
            class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('container.dismiss')"
            :aria-label="t('container.dismiss')"
            @click="dismiss()"
          >
            <X :size="11" />
          </button>
        </div>

        <ContainerOverview
          v-show="view === 'overview'"
          :tab="props.tab"
          :session="session"
          @show-state="showState"
          @show-images="view = 'images'"
        />
        <!-- v-show, not v-if: a pull in progress and a filter being typed survive a view switch. -->
        <ContainerListView
          v-show="view === 'containers'"
          :tab="props.tab"
          :session="session"
          :image-filter="imageFilter"
          :state-filter="stateFilter"
          @clear-image-filter="imageFilter = null"
          @clear-state-filter="stateFilter = null"
          @filter-state="stateFilter = $event"
        />
        <ContainerImageView
          v-show="view === 'images'"
          :tab="props.tab"
          :session="session"
          @view-related="showRelatedContainers"
        />
      </div>
    </div>
  </section>
</template>
