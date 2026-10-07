<script setup lang="ts">
/**
 * One setting drawn as a field: its name above, the control that changes it under.
 *
 * The other shape a setting takes on this page is a card — a bordered row with a name, a one-line
 * summary and the control on the right. Which shape a setting gets is decided in `rows.ts`, and it
 * is about how much there is to say: a language, a colour and a size are each a word and a control,
 * and three cards saying that would push everything the page is actually about below the fold.
 *
 * What a field gives up is the summary line, not the explanation: the question mark beside the name
 * opens the same text a card's does, and the search still finds the row by either line.
 *
 * The one exception is the row of four at the foot of the appearance page: there, a quarter of the
 * page's width holds nothing but a control, and four controls with no words between them are four
 * values nobody can tell apart. Those fields pass their summary in, and it is drawn under the
 * control — the only grid that draws one.
 */
import SettingsHelp from "@/components/settings/SettingsHelp.vue";

const props = defineProps<{
  /** The setting's name, which is also the control's accessible name. */
  title: string;
  /** The whole explanation, behind the question mark. */
  help: string;
  /** The one-line summary, drawn only where the grid asks for it. */
  summary?: string;
}>();
</script>

<template>
  <div class="min-w-0 space-y-1.5">
    <div class="flex min-w-0 items-center gap-1">
      <span class="truncate text-sm font-medium leading-tight">{{ props.title }}</span>
      <SettingsHelp :title="props.title" :help="props.help" />
    </div>
    <slot />
    <p v-if="props.summary" class="text-xs leading-relaxed text-muted-foreground">{{ props.summary }}</p>
  </div>
</template>
