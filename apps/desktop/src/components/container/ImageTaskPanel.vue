<script setup lang="ts">
/**
 * The pulls and pushes that are still going, and the ones that finished a moment ago.
 *
 * A list rather than a dialog, because a pull is minutes long and several may run at once: what a
 * person needs is to see them side by side and to be able to stop one without losing sight of the
 * others. Each row's raw output expands under it, capped, for the moment a transfer fails and the
 * sentence that says why is in the middle of it.
 *
 * The percentage is the store's reading of the runtime's own lines; while nothing measurable has
 * arrived the bar is drawn as waiting rather than as zero.
 */
import { ArrowDownToLine, ArrowUpFromLine, Eraser, ScrollText, Square, Trash2 } from "@lucide/vue";
import { computed, onBeforeUnmount, ref } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import {
  imageProgressPercent,
  layerCounts,
  useContainerStore,
  type ImageTask,
  type ImageTaskStatus,
} from "@/stores/container";

const props = defineProps<{ tabId: string; tasks: ImageTask[] }>();

const { t } = useI18n();
const containers = useContainerStore();

const expanded = ref<string | null>(null);
const now = ref(Date.now());
const ticker = window.setInterval(() => {
  now.value = Date.now();
}, 1000);

onBeforeUnmount(() => {
  window.clearInterval(ticker);
});

const running = computed(() => props.tasks.filter((task) => task.status === "running"));
const settled = computed(() => props.tasks.filter((task) => task.status !== "running"));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

function statusLabel(status: ImageTaskStatus): string {
  switch (status) {
    case "running":
      return t("container.tasks.running");
    case "done":
      return t("container.tasks.done");
    case "failed":
      return t("container.tasks.failed");
    case "stopped":
      return t("container.tasks.stopped");
  }
}

function statusClass(status: ImageTaskStatus): string {
  switch (status) {
    case "done":
      return "text-success";
    case "failed":
      return "text-destructive";
    case "stopped":
      return "text-muted-foreground";
    default:
      return "text-muted-foreground";
  }
}

function percent(task: ImageTask): number | null {
  if (task.status === "done") {
    return 100;
  }
  return imageProgressPercent(task.layers);
}

function layersLabel(task: ImageTask): string {
  const counts = layerCounts(task.layers);
  return t("container.tasks.layers", { done: String(counts.done), total: String(counts.total) });
}

function elapsed(task: ImageTask): string {
  const end = task.endedAt > 0 ? task.endedAt : now.value;
  const seconds = Math.max(0, Math.round((end - task.startedAt) / 1000));
  if (seconds < 60) {
    return t("container.tasks.elapsedSeconds", { seconds: String(seconds) });
  }
  return t("container.tasks.elapsedMinutes", {
    minutes: String(Math.floor(seconds / 60)),
    seconds: String(seconds % 60),
  });
}

function failureText(task: ImageTask): string | null {
  if (task.streamError !== null) {
    return task.streamError;
  }
  if (task.failure !== null) {
    return describeShellError(translate, task.failure).message;
  }
  return null;
}

function toggleLogs(task: ImageTask): void {
  expanded.value = expanded.value === task.id ? null : task.id;
}

function stop(task: ImageTask): void {
  void containers.stopTask(props.tabId, task.id);
}

function clear(task: ImageTask): void {
  containers.clearTask(props.tabId, task.id);
  if (expanded.value === task.id) {
    expanded.value = null;
  }
}

function clearFinished(): void {
  containers.clearFinishedTasks(props.tabId);
  expanded.value = null;
}
</script>

<template>
  <section class="shrink-0 border-t border-border" :aria-label="t('container.tasks.title')">
    <header class="flex items-center gap-2 px-2.5 py-1">
      <h3 class="min-w-0 flex-1 truncate text-[10px] font-medium tracking-wide text-muted-foreground uppercase">
        {{ t("container.tasks.title") }}
      </h3>
      <span class="shrink-0 font-mono text-[10px] text-muted-foreground">
        {{ running.length }} / {{ props.tasks.length }}
      </span>
      <button
        v-if="settled.length > 0"
        type="button"
        class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('container.tasks.clearFinished')"
        :aria-label="t('container.tasks.clearFinished')"
        @click="clearFinished()"
      >
        <Eraser :size="11" />
      </button>
    </header>

    <ul class="max-h-44 overflow-y-auto">
      <li v-for="task in props.tasks" :key="task.id" class="border-t border-border/60">
        <div class="flex items-center gap-2 px-2.5 py-1">
          <ArrowDownToLine v-if="task.kind === 'pull'" :size="11" class="shrink-0 text-success" />
          <ArrowUpFromLine v-else :size="11" class="shrink-0 text-primary" />
          <span class="min-w-0 flex-1 truncate font-mono text-[11px]" :title="task.image">{{ task.image }}</span>

          <span v-if="percent(task) !== null && task.status === 'running'" class="w-32 shrink-0">
            <span class="block h-1.5 overflow-hidden rounded-full bg-muted">
              <span
                class="block h-full rounded-full bg-primary transition-[width] duration-300"
                :style="{ width: `${percent(task)}%` }"
              ></span>
            </span>
          </span>
          <span v-else-if="task.status === 'running'" class="w-32 shrink-0">
            <span class="block h-1.5 w-1/3 animate-pulse rounded-full bg-primary"></span>
          </span>
          <span v-else class="w-32 shrink-0 text-right font-mono text-[10px] text-muted-foreground">
            {{ percent(task) === null ? "" : `${percent(task)}%` }}
          </span>

          <span class="w-20 shrink-0 truncate text-right font-mono text-[10px] text-muted-foreground">
            {{ layersLabel(task) }}
          </span>
          <span class="w-16 shrink-0 text-right text-[10px]" :class="statusClass(task.status)">
            {{ statusLabel(task.status) }}
          </span>
          <span class="w-14 shrink-0 text-right font-mono text-[10px] text-muted-foreground">{{ elapsed(task) }}</span>

          <button
            type="button"
            class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('container.tasks.logs')"
            :aria-label="t('container.tasks.logs')"
            @click="toggleLogs(task)"
          >
            <ScrollText :size="11" />
          </button>
          <button
            v-if="task.status === 'running'"
            type="button"
            class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('container.tasks.stop')"
            :aria-label="t('container.tasks.stop')"
            @click="stop(task)"
          >
            <Square :size="10" />
          </button>
          <button
            v-else
            type="button"
            class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('container.tasks.clear')"
            :aria-label="t('container.tasks.clear')"
            @click="clear(task)"
          >
            <Trash2 :size="11" />
          </button>
        </div>

        <p v-if="failureText(task)" class="px-2.5 pb-1 font-mono text-[10px] break-all text-destructive select-text">
          {{ failureText(task) }}
        </p>

        <pre
          v-if="expanded === task.id"
          class="max-h-32 overflow-auto border-t border-border/60 bg-background px-2.5 py-1.5 font-mono text-[10px] whitespace-pre-wrap select-text"
          >{{ task.lines.join("\n") }}</pre
        >
      </li>
    </ul>
  </section>
</template>
