<script setup lang="ts">
/**
 * One setting, drawn as a row: what it is on the left, the control that changes it on the right.
 *
 * The row is the same shape for every setting on purpose. A page of settings is read by scanning
 * down the left edge for the thing to change and then across to the control, and that only works if
 * every row puts those two in the same places — which is also why the control arrives as a slot:
 * the row decides where a control goes, the page decides which control it is.
 *
 * The long explanation is not on the row. It sits behind the question mark beside the title, where
 * it is there for whoever wants it and does not make every row four lines tall — the one-line
 * summary under the title is what a person reads to tell two similar rows apart.
 */
import { CircleHelp } from "@lucide/vue";
import { TooltipContent, TooltipPortal, TooltipRoot, TooltipTrigger } from "reka-ui";
import { useI18n } from "vue-i18n";

const props = defineProps<{
  title: string;
  /** One line under the title. */
  summary: string;
  /** The whole explanation, shown from the question mark. */
  help: string;
  /** Whether this row is a search hit, which is what plays the highlight once. */
  found?: boolean;
}>();

const { t } = useI18n();
</script>

<template>
  <div
    class="flex items-start justify-between gap-4 rounded-md border border-border bg-muted/20 px-3 py-2.5 transition-colors hover:bg-muted"
    :class="props.found ? 'settings-found' : undefined"
  >
    <div class="min-w-0">
      <div class="flex items-center gap-1.5">
        <span class="text-sm font-medium">{{ props.title }}</span>
        <!--
          A button rather than a bare icon: the explanation has to be reachable without a mouse, and
          a focusable trigger is what a tooltip opens on. Its name says which setting it explains,
          because a screen reader hears a page full of "question mark" otherwise.
        -->
        <TooltipRoot>
          <TooltipTrigger
            class="grid size-4 shrink-0 place-items-center rounded-full text-muted-foreground transition-colors hover:text-foreground focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none"
            :aria-label="t('settings.help.open', { setting: props.title })"
          >
            <CircleHelp :size="13" />
          </TooltipTrigger>
          <TooltipPortal>
            <TooltipContent
              side="top"
              :side-offset="6"
              class="z-50 max-w-80 rounded-lg border border-border bg-card px-3 py-2 text-xs leading-relaxed text-card-foreground shadow-xl"
            >
              {{ props.help }}
            </TooltipContent>
          </TooltipPortal>
        </TooltipRoot>
      </div>
      <p class="mt-0.5 text-xs text-muted-foreground">{{ props.summary }}</p>
    </div>

    <div class="shrink-0 pt-0.5">
      <slot />
    </div>
  </div>
</template>
