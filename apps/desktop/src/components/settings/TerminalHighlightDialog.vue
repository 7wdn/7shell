<script setup lang="ts">
/**
 * One highlight rule, written or changed.
 *
 * The pattern is checked here because this is the only place that knows whether it is meant as a
 * regular expression: a plain word is always a valid pattern, and an expression that does not
 * compile is a rule the terminal could not apply. The colour is a native picker, because a colour
 * is chosen by looking at it.
 */
import { X } from "@lucide/vue";
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { importedSchemeId } from "@/lib/terminal/importScheme";
import {
  DEFAULT_TERMINAL_HIGHLIGHT_COLOR,
  TERMINAL_HIGHLIGHT_PATTERN_MAX,
  type TerminalHighlightRule,
} from "@/types/app";

const props = defineProps<{
  open: boolean;
  /** The rule being changed; nothing means a new one. */
  rule: TerminalHighlightRule | null;
  /** Ids the new rule must not take. */
  takenIds: readonly string[];
}>();

const emit = defineEmits<{
  save: [rule: TerminalHighlightRule];
  "update:open": [value: boolean];
}>();

const { t } = useI18n();

const draft = ref({
  pattern: "",
  color: DEFAULT_TERMINAL_HIGHLIGHT_COLOR,
  isRegex: false,
  caseSensitive: false,
});

const error = computed<"tooLong" | "invalidRegex" | null>(() => {
  const pattern = draft.value.pattern;
  if (pattern.length > TERMINAL_HIGHLIGHT_PATTERN_MAX) {
    return "tooLong";
  }
  if (draft.value.isRegex && pattern.trim()) {
    try {
      new RegExp(pattern, draft.value.caseSensitive ? "u" : "iu");
    } catch {
      return "invalidRegex";
    }
  }
  return null;
});

const canSave = computed(() => draft.value.pattern.trim() !== "" && error.value === null);

watch(
  () => [props.open, props.rule] as const,
  ([open, rule]) => {
    if (!open) {
      return;
    }
    draft.value = {
      pattern: rule?.pattern ?? "",
      color: rule?.color ?? DEFAULT_TERMINAL_HIGHLIGHT_COLOR,
      isRegex: rule?.isRegex ?? false,
      caseSensitive: rule?.caseSensitive ?? false,
    };
  },
  { immediate: true },
);

function submit(): void {
  if (!canSave.value) {
    return;
  }
  const pattern = draft.value.pattern.trim();
  emit("save", {
    id: props.rule?.id ?? importedSchemeId(pattern, new Set(props.takenIds)),
    pattern,
    color: draft.value.color,
    isRegex: draft.value.isRegex,
    caseSensitive: draft.value.caseSensitive,
  });
  emit("update:open", false);
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <header class="flex items-center justify-between border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">
            {{ props.rule ? t("settings.highlights.editTitle") : t("settings.highlights.addTitle") }}
          </DialogTitle>
          <DialogDescription class="sr-only">{{ t("settings.highlights.hint") }}</DialogDescription>
          <DialogClose
            class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('common.action.close')"
          >
            <X :size="15" />
          </DialogClose>
        </header>

        <form class="space-y-4 px-4 py-4" @submit.prevent="submit()">
          <div>
            <label class="block text-xs font-medium" for="highlight-pattern">{{
              t("settings.highlights.pattern")
            }}</label>
            <input
              id="highlight-pattern"
              v-model="draft.pattern"
              spellcheck="false"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus:border-primary focus:outline-none"
              :placeholder="t('settings.highlights.patternPlaceholder')"
              :maxlength="TERMINAL_HIGHLIGHT_PATTERN_MAX + 1"
            />
            <p v-if="error === 'tooLong'" class="mt-1 text-xs text-destructive">
              {{ t("settings.highlights.tooLong", { max: TERMINAL_HIGHLIGHT_PATTERN_MAX }) }}
            </p>
            <p v-else-if="error === 'invalidRegex'" class="mt-1 text-xs text-destructive">
              {{ t("settings.highlights.invalidRegex") }}
            </p>
          </div>

          <div class="flex items-center gap-3">
            <label class="block text-xs font-medium" for="highlight-colour">{{ t("settings.highlights.color") }}</label>
            <input
              id="highlight-colour"
              v-model="draft.color"
              type="color"
              class="h-7 w-12 cursor-pointer rounded border border-border bg-transparent p-0.5"
            />
            <span class="font-mono text-xs text-muted-foreground">{{ draft.color }}</span>
          </div>

          <label class="flex items-center gap-2 text-sm">
            <input v-model="draft.isRegex" type="checkbox" class="size-3.5 accent-primary" />
            {{ t("settings.highlights.regex") }}
          </label>
          <label class="flex items-center gap-2 text-sm">
            <input v-model="draft.caseSensitive" type="checkbox" class="size-3.5 accent-primary" />
            {{ t("settings.highlights.caseSensitive") }}
          </label>

          <div class="flex justify-end gap-2 pt-1">
            <DialogClose class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent">
              {{ t("common.action.cancel") }}
            </DialogClose>
            <button
              type="submit"
              class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
              :disabled="!canSave"
            >
              {{ t("settings.highlights.save") }}
            </button>
          </div>
        </form>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
