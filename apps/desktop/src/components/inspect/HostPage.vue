<script setup lang="ts">
/**
 * The frame every host page shares: whether it is being read, and why not when it is not.
 *
 * Which machine a page is about is the tab's business rather than something the page writes across
 * its own top: the tab carries the page and the host, and the monitor panel carries the address and
 * the state of the connection. A strip repeating them was removed for saying the same thing twice —
 * and the word it ended with, "live", for saying nothing at all: a page reads while it is on screen,
 * which is what its numbers arriving already shows.
 *
 * What is left is what a page cannot draw for itself: the two states it can be in, and the sentence
 * that says which of them it is.
 *
 * A page reads through whatever terminal its session has open; it holds no connection of its own.
 * That is why it can be left open across a reconnect, and why "not connected" here is a state
 * rather than a failure: a page whose host is not connected is a page waiting, not a page broken.
 */
import { CircleAlert } from "@lucide/vue";
import { computed, onBeforeUnmount, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { useMonitorStore } from "@/stores/monitor";
import type { Session } from "@/types/session";

const props = defineProps<{
  session: Session;
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
