<script setup lang="ts">
/**
 * The mark of the system a host said it is, drawn to sit in a line of text.
 *
 * It draws what there is and nothing else: a host that never answered, or answered with something
 * this build has no mark for, renders no element at all — the caller decides what stands in its
 * place, because what a row falls back to is about the row rather than about the system. Colours
 * stay as the drawings carry them, which is the point of a mark: it is recognised by its colours,
 * and a monochrome version would be a different picture for the sake of a theme.
 */
import { computed } from "vue";

import { systemMark, systemMarkup } from "@/lib/system/icons";
import type { HostSystem } from "@/types/system";

const props = defineProps<{
  /** What the host said it is. "Nothing to draw" is a normal answer rather than a failure. */
  readonly system: HostSystem | null;
  /**
   * The box the mark is drawn in, in pixels.
   *
   * Bigger than the stroke icons it stands beside, on purpose: a mark is a picture that brings its
   * own white space with it, so a box the size of a lucide glyph reads as a visibly smaller picture
   * — which is exactly how it looked in a session row at thirteen. Sixteen is what makes the two
   * read as the same size next to the same two lines of text.
   */
  readonly size?: number;
  /** What to call the mark for a reader who cannot see it. Absent means it is decorative. */
  readonly label?: string | null;
}>();

const markup = computed(() => systemMarkup(systemMark(props.system)));
const box = computed(() => `${props.size ?? 16}px`);
</script>

<template>
  <span
    v-if="markup"
    class="os-mark inline-flex shrink-0 items-center justify-center"
    :style="{ width: box, height: box }"
    :role="props.label ? 'img' : undefined"
    :aria-label="props.label ?? undefined"
    :aria-hidden="props.label ? undefined : 'true'"
    v-html="markup"
  />
</template>

<style scoped>
/* The box speaks last: the files carry pixel sizes of their own, and a 64px mark in a 13px row is
   what happens when a download's own `width` wins. */
.os-mark :deep(svg) {
  display: block;
  width: 100%;
  height: 100%;
}
</style>
