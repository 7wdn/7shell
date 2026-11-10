<script setup lang="ts">
/**
 * The frame every container dialog is drawn in: a title, the body the caller supplies, and its own
 * footer.
 *
 * It exists because four dialogs share one shape, and the shape is where the mistakes live — a
 * body that scrolls while the title stays put, a footer that does not move when the body is long, a
 * question that can be dismissed while the action behind it runs. Written once, the Escape key and
 * the click outside behave the same way in all four.
 *
 * No text is written here: the title and the footer's words arrive from the caller, which is where
 * the translation is.
 */
import { X } from "@lucide/vue";
import { DialogContent, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { useI18n } from "vue-i18n";

const props = withDefaults(
  defineProps<{
    open: boolean;
    title: string;
    /** A sentence under the title, when the title alone is not the whole instruction. */
    description?: string;
    /** While the action behind the dialog runs, it refuses to be dismissed. */
    busy?: boolean;
  }>(),
  { description: undefined, busy: false },
);

const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();

/** Escape and a click outside are a cancel, unless the work has already started. */
function refuseWhileBusy(event: Event): void {
  if (props.busy) {
    event.preventDefault();
  }
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[min(80vh,44rem)] w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl select-none focus:outline-none"
        @interact-outside="refuseWhileBusy"
        @escape-key-down="refuseWhileBusy"
      >
        <header class="flex shrink-0 items-center justify-between gap-2 border-b border-border px-4 py-2.5">
          <DialogTitle class="min-w-0 truncate text-sm font-medium">{{ props.title }}</DialogTitle>
          <button
            type="button"
            class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-not-allowed disabled:opacity-40"
            :title="t('common.action.close')"
            :aria-label="t('common.action.close')"
            :disabled="props.busy"
            @click="emit('update:open', false)"
          >
            <X :size="12" />
          </button>
        </header>

        <div class="min-h-0 flex-1 overflow-y-auto px-4 py-3 select-text">
          <p v-if="props.description" class="mb-3 text-[11px] leading-relaxed text-muted-foreground">
            {{ props.description }}
          </p>
          <slot />
        </div>

        <footer
          class="flex shrink-0 items-center justify-end gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-2.5"
        >
          <slot name="footer" />
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
