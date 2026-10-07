<script setup lang="ts">
/**
 * The question mark beside a setting's name, which is where the whole explanation lives.
 *
 * One component rather than the markup twice: the settings page draws each setting in two shapes —
 * a card with room for a one-line summary, and a bare field in a grid with room for nothing but its
 * name — and the long explanation has to open the same way from both, including for somebody who
 * never touches a mouse.
 */
import { CircleHelp } from "@lucide/vue";
import { TooltipContent, TooltipPortal, TooltipRoot, TooltipTrigger } from "reka-ui";
import { useI18n } from "vue-i18n";

const props = defineProps<{
  /** The setting's own name, which is what the explanation is an answer about. */
  title: string;
  /** The whole explanation. */
  help: string;
}>();

const { t } = useI18n();
</script>

<template>
  <!--
    A button rather than a bare icon: the explanation has to be reachable without a mouse, and a
    focusable trigger is what a tooltip opens on. Its name says which setting it explains, because a
    screen reader hears a page full of "question mark" otherwise.
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
</template>
