<script setup lang="ts">
/**
 * One piece of the chat: a question, a paragraph of the answer, or the model's aside.
 *
 * Three shapes rather than three components, because they are the same thing said by different
 * voices and the difference is a label and a tone of voice — not a layout. What the block is about
 * is carried by `role`, and the only role with anything extra is a question, whose terminal lines
 * are a fact about it rather than a paragraph after it.
 *
 * The aside starts folded. It is the model's reasoning rather than its answer, and a panel that
 * opened with a page of it would push the answer somebody asked for below the fold.
 */
import { ChevronRight } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

const props = withDefaults(
  defineProps<{
    role: "user" | "assistant" | "thinking";
    text: string;
    /** Still arriving: the block says so, and the last line ends with a caret. */
    streaming?: boolean;
    /** For a question: how many terminal lines travelled with it. Zero means none did. */
    contextLines?: number;
  }>(),
  { streaming: false, contextLines: 0 },
);

const { t } = useI18n();

const label = computed(() => {
  if (props.role === "user") {
    return t("ai.message.you");
  }
  return props.role === "thinking" ? t("ai.message.thinking") : "";
});

const attached = computed(() =>
  (props.contextLines ?? 0) > 0
    ? t("ai.message.context", { count: props.contextLines ?? 0 })
    : t("ai.message.contextNone"),
);
</script>

<template>
  <!--
    The aside is a disclosure rather than a block: closed it costs one line, and open it is the same
    text the answer above or below it is made of.
  -->
  <details v-if="role === 'thinking'" class="group rounded-md border border-border/60 bg-muted/20 px-2.5 py-1.5">
    <summary class="flex cursor-pointer list-none items-center gap-1.5 text-[11px] text-muted-foreground select-none">
      <ChevronRight :size="11" class="shrink-0 transition-transform group-open:rotate-90" />
      <span class="font-medium">{{ label }}</span>
      <span v-if="streaming" class="animate-pulse">{{ t("ai.message.streaming") }}</span>
    </summary>
    <p class="mt-1.5 text-[11px] leading-relaxed whitespace-pre-wrap break-words text-muted-foreground select-text">
      {{ text }}
    </p>
  </details>

  <div v-else-if="role === 'user'" class="space-y-1">
    <div class="flex items-baseline gap-2">
      <span class="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">{{ label }}</span>
      <span
        class="truncate text-[10px] text-muted-foreground"
        :title="contextLines && contextLines > 0 ? t('ai.context.title') : undefined"
      >
        {{ attached }}
      </span>
    </div>
    <p
      class="rounded-md border border-border bg-muted/30 px-2.5 py-2 text-xs leading-relaxed whitespace-pre-wrap break-words select-text"
    >
      {{ text }}
    </p>
  </div>

  <div v-else class="text-xs leading-relaxed whitespace-pre-wrap break-words select-text">
    {{ text }}<span v-if="streaming" class="ml-0.5 inline-block translate-y-px animate-pulse">▌</span>
  </div>
</template>
