<script setup lang="ts">
/**
 * The frame every host page shares: which machine it is about, whether it is being read, and why
 * not when it is not.
 *
 * The three pages differ only in what they list, so the parts that are easy to get subtly different
 * — the title, the two states a page can be in, the sentence that says which of them it is — live
 * here once. What a page provides is a body that draws dashes where its own section is missing.
 *
 * A page reads through whatever terminal its session has open; it holds no connection of its own.
 * That is why it can be left open across a reconnect, and why "not connected" here is a state
 * rather than a failure: a page whose host is not connected is a page waiting, not a page broken.
 */
import { CircleAlert, LoaderCircle } from "@lucide/vue";
import { computed, onBeforeUnmount, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { useMonitorStore } from "@/stores/monitor";
import type { InspectView } from "@/stores/terminals";
import type { Session } from "@/types/session";

const props = defineProps<{
  session: Session;
  view: InspectView;
  /** Whether this page is the tab on screen. A hidden page samples nothing and shows nothing. */
  active: boolean;
}>();

const { t } = useI18n();
const monitor = useMonitorStore();

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/**
 * Whether the numbers on this page are about this host.
 *
 * Both halves matter: a page behind another tab must not be read at all, and a page that is on
 * screen while its session has no connection has nothing to be about. The sample is asked for by
 * the session it came from rather than by "is there a sample", because switching from one host's
 * tab to another's would otherwise draw the first host's numbers under the second host's name for
 * the moment before the next sample arrives.
 */
const watching = computed(() => props.active && monitor.target?.sessionId === props.session.id);

/** Watching, and with something to show that is about this host: what a body draws. */
const live = computed(() => props.active && monitor.sampleSessionId === props.session.id);

const title = computed(() => {
  switch (props.view) {
    case "system":
      return t("inspect.system.title");
    case "processes":
      return t("inspect.processes.title");
    default:
      return t("inspect.network.title");
  }
});

const failure = computed(() => (monitor.failure ? describeShellError(translate, monitor.failure) : null));

// The page is what asks for samples while it is the tab on screen; the panel asks for its own.
watch(
  () => props.active,
  (active) => (active ? monitor.start() : monitor.stop()),
  { immediate: true },
);
onBeforeUnmount(() => {
  if (props.active) {
    monitor.stop();
  }
});
</script>

<template>
  <section class="flex min-h-0 flex-1 flex-col overflow-hidden">
    <header class="flex shrink-0 flex-wrap items-baseline gap-2 border-b border-border px-4 py-2">
      <h2 class="text-sm font-medium">{{ title }}</h2>
      <span class="font-mono text-xs text-muted-foreground">{{ props.session.name }}</span>
      <span class="font-mono text-xs text-muted-foreground">
        {{ props.session.user }}@{{ props.session.host }}:{{ props.session.port }}
      </span>
      <span class="flex-1"></span>
      <span v-if="watching" class="flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <LoaderCircle v-if="monitor.sampling" :size="11" class="animate-spin" />
        {{ t("inspect.reading") }}
      </span>
    </header>

    <!--
      A flex column rather than a scrolling box: a page may want to split itself — the network
      page keeps its sockets and their connections apart — so the frame gives it the room and
      the page decides what scrolls inside it.
    -->
    <div class="flex min-h-0 flex-1 flex-col overflow-hidden px-4 py-3">
      <p v-if="!watching" class="text-xs leading-relaxed text-muted-foreground">
        <span class="text-foreground">{{ t("inspect.disconnected.title") }}</span> —
        {{ t("inspect.disconnected.hint", { name: props.session.name }) }}
      </p>

      <div v-else-if="failure" class="mb-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2">
        <p class="flex items-start gap-1.5 text-xs">
          <CircleAlert :size="13" class="mt-0.5 shrink-0 text-destructive" />
          <span>{{ failure.message }}</span>
        </p>
        <p class="mt-0.5 pl-[19px] font-mono text-[10px] text-muted-foreground">{{ failure.code }}</p>
      </div>

      <slot :live="live" />
    </div>
  </section>
</template>
