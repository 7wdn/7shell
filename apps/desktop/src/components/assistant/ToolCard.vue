<script setup lang="ts">
/**
 * One tool call, as a card.
 *
 * The card exists because a tool call is the one part of an answer nobody can read as prose: it has
 * a name, a status, a duration and a result that may be a wall of text. Three things follow, and each
 * is something the card would otherwise get wrong:
 *
 * 1. **The status is drawn, not implied.** A call that is still running and one that failed are two
 *    different answers, and a card that only appeared at the end would leave the person looking at a
 *    panel that seemed to have stopped.
 * 2. **The result is bounded before it is drawn.** The host truncates to the settings' result limit;
 *    the store bounds again to what a narrow panel can show, and the card says when either bound
 *    cut something. A preview nobody can tell was cut is a preview that lies about the answer.
 * 3. **The input is folded away.** It is what the tool was asked to do, which is worth seeing when a
 *    call went wrong and noise when it did not.
 */
import { ChevronRight } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { AssistantToolBlock } from "@/stores/assistant";

const props = defineProps<{
  block: AssistantToolBlock;
}>();

const { t } = useI18n();

const statusLabel = computed(() => {
  switch (props.block.status) {
    case "running":
      return t("ai.tool.running");
    case "ok":
      return t("ai.tool.ok");
    default:
      return t("ai.tool.failed");
  }
});

const statusClass = computed(() => {
  switch (props.block.status) {
    case "running":
      return "text-muted-foreground";
    case "ok":
      return "text-success";
    default:
      return "text-destructive";
  }
});

const dotClass = computed(() => {
  switch (props.block.status) {
    case "running":
      return "bg-muted-foreground animate-pulse";
    case "ok":
      return "bg-success";
    default:
      return "bg-destructive";
  }
});

/** The input as text, folded away: a JSON wall nobody reads unless a call went wrong. */
const inputText = computed(() => {
  if (props.block.input === null || props.block.input === undefined) {
    return "";
  }
  try {
    return JSON.stringify(props.block.input, null, 2);
  } catch {
    return String(props.block.input);
  }
});
</script>

<template>
  <div class="rounded-md border border-border bg-card/40">
    <div class="flex items-center gap-2 px-2.5 py-1.5">
      <span class="size-1.5 shrink-0 rounded-full" :class="dotClass" aria-hidden="true"></span>
      <span class="min-w-0 flex-1 truncate font-mono text-[11px]">{{ block.tool }}</span>
      <span class="shrink-0 text-[10px]" :class="statusClass">{{ statusLabel }}</span>
      <span v-if="block.durationMs !== undefined" class="shrink-0 font-mono text-[10px] text-muted-foreground">
        {{ t("ai.tool.duration", { ms: block.durationMs }) }}
      </span>
    </div>

    <div v-if="block.preview" class="border-t border-border/60 px-2.5 py-1.5">
      <pre
        class="max-h-48 overflow-auto font-mono text-[10px] leading-relaxed whitespace-pre-wrap break-words select-text"
        >{{ block.preview }}</pre
      >
      <p v-if="block.truncated" class="mt-1 text-[10px] text-muted-foreground">{{ t("ai.tool.truncated") }}</p>
    </div>
    <p
      v-else-if="block.status !== 'running'"
      class="border-t border-border/60 px-2.5 py-1.5 text-[10px] text-muted-foreground"
    >
      {{ t("ai.tool.empty") }}
    </p>

    <p
      v-if="block.status === 'failed' && block.code"
      class="border-t border-border/60 px-2.5 py-1 font-mono text-[10px] text-destructive"
    >
      {{ block.code }}
    </p>

    <details v-if="inputText" class="group border-t border-border/60 px-2.5 py-1">
      <summary class="flex cursor-pointer list-none items-center gap-1 text-[10px] text-muted-foreground select-none">
        <ChevronRight :size="10" class="shrink-0 transition-transform group-open:rotate-90" />
        {{ t("ai.tool.input") }}
      </summary>
      <pre
        class="mt-1 max-h-40 overflow-auto font-mono text-[10px] leading-relaxed whitespace-pre-wrap break-words select-text"
        >{{ inputText }}</pre
      >
    </details>
  </div>
</template>
