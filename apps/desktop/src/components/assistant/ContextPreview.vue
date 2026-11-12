<script setup lang="ts">
/**
 * What the next question will carry from the terminal, shown before it is asked.
 *
 * The preview is the point of the whole context feature: remote output leaving the machine is a
 * decision, and a decision needs the exact text rather than a description of it. So the box below
 * draws `context.text` — the string the store hands to `send`, already stripped, redacted and
 * capped — and nothing here regenerates or abbreviates it.
 *
 * The switch and the number are the settings, read from the same place the panel reads them. They
 * are drawn here rather than only on the settings page because the moment somebody wants to change
 * them is the moment they are looking at what would travel.
 */
import { RotateCw } from "@lucide/vue";
import { useI18n } from "vue-i18n";

import Switch from "@/components/ui/Switch.vue";
import type { AssistantContextSnapshot } from "@/stores/assistant";
import { AI_CONTEXT_LINES_MAX, AI_CONTEXT_LINES_MIN } from "@/types/app";
import { AI_CONTEXT_MAX_CHARS, type AiContextResource } from "@/types/ai";

defineProps<{
  enabled: boolean;
  lines: number;
  context: AssistantContextSnapshot;
  /** How many lines the prepared text holds, which is what "ready" counts. */
  lineCount: number;
  /**
   * The external-server resources attached to the next question.
   *
   * Drawn whether or not the terminal half is switched on: these were chosen one at a time, and the
   * switch above is about reading a buffer rather than about what somebody attached on purpose.
   */
  attachments: readonly AiContextResource[];
}>();

const emit = defineEmits<{
  toggle: [enabled: boolean];
  lines: [value: number];
  refresh: [];
}>();

const { t } = useI18n();

function typed(event: Event): number {
  return Number((event.target as HTMLInputElement).value || "0");
}

const BUDGET = new Intl.NumberFormat(undefined, { maximumFractionDigits: 0 }).format(AI_CONTEXT_MAX_CHARS);
</script>

<template>
  <section class="shrink-0 space-y-1.5 border-t border-border px-3 py-2">
    <div class="flex items-center gap-2">
      <h3 class="min-w-0 flex-1 truncate text-[11px] font-medium">{{ t("ai.context.title") }}</h3>
      <span v-if="enabled && lineCount > 0" class="shrink-0 text-[10px] text-muted-foreground">
        {{ t("ai.context.attached", { count: lineCount }) }}
      </span>
      <Switch
        :model-value="enabled"
        :aria-label="t('ai.context.enabled')"
        @update:model-value="(value) => emit('toggle', value)"
      />
    </div>

    <div v-if="enabled" class="flex items-center gap-1.5">
      <label class="shrink-0 text-[10px] text-muted-foreground" for="assistant-context-lines">
        {{ t("ai.context.lines") }}
      </label>
      <input
        id="assistant-context-lines"
        type="number"
        class="w-14 rounded-md border border-border bg-background px-1.5 py-0.5 text-[11px] focus:outline-none select-text"
        :min="AI_CONTEXT_LINES_MIN"
        :max="AI_CONTEXT_LINES_MAX"
        step="1"
        :value="lines"
        @input="emit('lines', typed($event))"
      />
      <span class="min-w-0 flex-1 truncate text-[10px] text-muted-foreground">{{ t("ai.context.linesHint") }}</span>
      <button
        type="button"
        class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('ai.context.refresh')"
        :aria-label="t('ai.context.refresh')"
        @click="emit('refresh')"
      >
        <RotateCw :size="11" />
      </button>
    </div>

    <p v-if="!enabled || lines === 0" class="text-[10px] leading-relaxed text-muted-foreground">
      {{ t("ai.context.off") }}
    </p>
    <p v-else-if="context.text === ''" class="text-[10px] leading-relaxed text-muted-foreground">
      {{ t("ai.context.empty") }}
    </p>
    <template v-else>
      <p class="text-[10px] text-muted-foreground">{{ t("ai.context.preview") }}</p>
      <pre
        class="max-h-32 overflow-auto rounded-md border border-border bg-background/60 px-2 py-1.5 font-mono text-[10px] leading-relaxed whitespace-pre-wrap break-words select-text"
        >{{ context.text }}</pre
      >
    </template>

    <!--
      The attached resources, whole. This is the exact text the host hands to the provider, so it is
      drawn in full and scrolls rather than being shortened — a preview that abbreviated the thing
      it previews would be the one piece of context nobody could check.
    -->
    <div v-if="attachments.length > 0" data-context-attachments class="space-y-1">
      <p class="text-[10px] text-muted-foreground">
        {{ t("ai.context.resources", { count: attachments.length }) }}
      </p>
      <div
        v-for="attachment in attachments"
        :key="`${attachment.server}:${attachment.uri}`"
        data-context-attachment
        class="space-y-0.5 rounded-md border border-border bg-background/60 px-2 py-1.5"
      >
        <p class="font-mono text-[10px] break-all">
          <span class="text-muted-foreground">{{ t("ai.context.server", { server: attachment.server }) }}</span>
          {{ attachment.name }}
        </p>
        <p class="font-mono text-[10px] break-all text-muted-foreground">{{ attachment.uri }}</p>
        <pre
          class="max-h-40 overflow-auto rounded-sm border border-border bg-background/60 px-2 py-1 font-mono text-[10px] leading-relaxed whitespace-pre-wrap break-words select-text"
          >{{ attachment.text }}</pre
        >
      </div>
    </div>

    <p class="text-[10px] leading-relaxed text-muted-foreground">
      {{ t("ai.context.limits", { chars: BUDGET }) }}
    </p>
  </section>
</template>
