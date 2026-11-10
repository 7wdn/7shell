<script setup lang="ts">
/**
 * A pod's log, as it is written.
 *
 * The stream is opened per selection — container, how many lines to start with, whether it is the
 * previous instance — because those are properties of the request rather than of the view. The four
 * toggles that are about reading rather than about asking (timestamps, wrapping, holding the view,
 * clearing) change nothing on the server, so changing one does not restart the stream.
 *
 * Timestamps always travel from the API and are hidden by the view when the toggle is off: refetching
 * a stream to hide a column would lose every line that arrived in between.
 */
import { ArrowDownToLine, LoaderCircle, Pause, Play, Trash2, WrapText, X } from "@lucide/vue";
import { computed, nextTick, onBeforeUnmount, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { LOG_LIMIT } from "@/components/k8s/logs";
import { containerNamesOf, nameOf, namespaceOf, type K8sObject } from "@/components/k8s/objects";
import { usePodLogs } from "@/components/k8s/usePodLogs";
import Select, { type SelectOption } from "@/components/ui/Select.vue";
import { describeShellError } from "@/i18n/errors";
import { cn } from "@/lib/utils";

const props = defineProps<{
  open: boolean;
  connId: string | null;
  pod: K8sObject | null;
}>();

const emit = defineEmits<{ close: [] }>();

const { t } = useI18n();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const logs = usePodLogs();
const body = ref<HTMLElement | null>(null);
const container = ref("");
const tail = ref("500");
const previous = ref(false);
const timestamps = ref(true);
const wrap = ref(false);

const containers = computed(() => (props.pod ? containerNamesOf(props.pod) : []));
const containerOptions = computed<SelectOption[]>(() => containers.value.map((name) => ({ value: name, label: name })));
const tailOptions: SelectOption[] = [
  { value: "100", label: "100" },
  { value: "500", label: "500" },
  { value: "2000", label: "2000" },
];

const title = computed(() => (props.pod ? t("k8s.logs.title", { pod: nameOf(props.pod) }) : ""));

/**
 * Opens the stream for the current selection.
 *
 * The namespace comes from the pod rather than from the picker: a log request names one object, and
 * an all-namespaces view has no namespace of its own to send.
 */
function restart(): void {
  const pod = props.pod;
  if (!props.open || pod === null || props.connId === null || container.value === "") {
    logs.stop();
    return;
  }
  void logs.start(props.connId, namespaceOf(pod), nameOf(pod), {
    container: container.value,
    tail: Number(tail.value),
    timestamps: true,
    previous: previous.value,
  });
}

/** The first container is what a new pod opens on; an explicit pick afterwards is kept. */
watch(
  () => props.pod,
  (pod) => {
    container.value = pod ? (containerNamesOf(pod)[0] ?? "") : "";
  },
  { immediate: true },
);

/** A stream restarts when the pod, the container, the tail or the previous flag changes. */
watch(
  () => [props.open, props.pod, container.value, tail.value, previous.value],
  () => {
    restart();
  },
  { immediate: true },
);

onBeforeUnmount(() => {
  logs.close();
});

/** Following the newest line, unless somebody is reading back through the buffer. */
watch(
  () => logs.lines.value.length,
  async () => {
    if (logs.paused.value) {
      return;
    }
    await nextTick();
    if (body.value) {
      body.value.scrollTop = body.value.scrollHeight;
    }
  },
);

function hold(): void {
  logs.paused.value = !logs.paused.value;
  if (!logs.paused.value && body.value) {
    body.value.scrollTop = body.value.scrollHeight;
  }
}

function close(): void {
  logs.close();
  emit("close");
}
</script>

<template>
  <aside
    v-if="props.open && props.pod"
    class="absolute inset-y-0 right-0 z-20 flex w-[min(46rem,100%)] flex-col border-l border-border bg-card shadow-xl"
    :aria-label="title"
  >
    <header class="flex shrink-0 items-center gap-2 border-b border-border px-3 py-2">
      <h2 class="min-w-0 flex-1 truncate text-xs font-medium" :title="title">{{ title }}</h2>
      <span v-if="logs.streaming.value" class="flex items-center gap-1 text-[11px] text-muted-foreground">
        <LoaderCircle :size="11" class="animate-spin" />
        {{ t("k8s.logs.live") }}
      </span>
      <button
        type="button"
        class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('k8s.logs.close')"
        :aria-label="t('k8s.logs.close')"
        @click="close"
      >
        <X :size="13" />
      </button>
    </header>

    <div class="flex shrink-0 flex-wrap items-center gap-1.5 border-b border-border px-3 py-1.5">
      <label class="flex items-center gap-1 text-[11px] text-muted-foreground">
        {{ t("k8s.logs.container") }}
        <Select
          v-model="container"
          :options="containerOptions"
          size="compact"
          :aria-label="t('k8s.logs.container')"
          trigger-class="font-mono"
        />
      </label>

      <label class="flex items-center gap-1 text-[11px] text-muted-foreground">
        {{ t("k8s.logs.tail") }}
        <Select
          v-model="tail"
          :options="tailOptions"
          size="compact"
          :aria-label="t('k8s.logs.tail')"
          trigger-class="font-mono"
        />
      </label>

      <label class="flex items-center gap-1 text-[11px] text-muted-foreground">
        <input v-model="previous" type="checkbox" class="size-3 accent-current" />
        {{ t("k8s.logs.previous") }}
      </label>

      <label class="flex items-center gap-1 text-[11px] text-muted-foreground">
        <input v-model="timestamps" type="checkbox" class="size-3 accent-current" />
        {{ t("k8s.logs.timestamps") }}
      </label>

      <label class="flex items-center gap-1 text-[11px] text-muted-foreground">
        <input v-model="wrap" type="checkbox" class="size-3 accent-current" />
        <WrapText :size="11" />
        {{ t("k8s.logs.wrap") }}
      </label>

      <button
        type="button"
        class="flex items-center gap-1 rounded-md border border-border px-1.5 py-0.5 text-[11px] transition-colors hover:bg-accent"
        @click="hold"
      >
        <Play v-if="logs.paused.value" :size="11" />
        <Pause v-else :size="11" />
        {{ logs.paused.value ? t("k8s.logs.resume") : t("k8s.logs.pause") }}
      </button>

      <button
        type="button"
        class="flex items-center gap-1 rounded-md border border-border px-1.5 py-0.5 text-[11px] transition-colors hover:bg-accent"
        @click="logs.clear()"
      >
        <Trash2 :size="11" />
        {{ t("k8s.logs.clear") }}
      </button>

      <span class="ml-auto flex items-center gap-1 font-mono text-[11px] text-muted-foreground">
        <ArrowDownToLine v-if="!logs.paused.value" :size="11" />
        {{ t("k8s.logs.buffer", { count: String(LOG_LIMIT) }) }}
      </span>
    </div>

    <div class="flex min-h-0 flex-1 flex-col">
      <p
        v-if="logs.failure.value"
        role="alert"
        class="shrink-0 border-b border-destructive/40 bg-destructive/10 px-3 py-2"
      >
        <span class="block text-xs">{{ describeShellError(translate, logs.failure.value).message }}</span>
        <span class="mt-0.5 block font-mono text-[11px] text-muted-foreground">
          {{ describeShellError(translate, logs.failure.value).code }}
        </span>
      </p>
      <p v-else-if="logs.ended.value && logs.endedWith.value !== ''" class="shrink-0 px-3 py-2 text-[11px]">
        {{ t("k8s.logs.ended", { reason: logs.endedWith.value }) }}
      </p>

      <div
        ref="body"
        class="min-h-0 flex-1 overflow-auto px-3 py-2 font-mono text-[11px] leading-relaxed select-text"
        :aria-label="t('k8s.logs.body')"
      >
        <p v-if="logs.lines.value.length === 0" class="text-muted-foreground">{{ t("k8s.logs.empty") }}</p>
        <div
          v-for="(line, index) in logs.lines.value"
          :key="index"
          :class="cn('flex gap-2', wrap ? 'whitespace-pre-wrap break-all' : 'whitespace-pre')"
        >
          <span v-if="timestamps && line.at !== ''" class="shrink-0 text-muted-foreground">{{ line.at }}</span>
          <span class="min-w-0">{{ line.text }}</span>
        </div>
      </div>
    </div>
  </aside>
</template>
