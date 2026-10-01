<script setup lang="ts">
/**
 * A row of bars, one per sample, for a value that has no meaning on its own.
 *
 * The bars share one scale, and the scale is the largest value across every series rather than
 * within each: two series drawn against their own maxima would cross each other the moment the
 * quieter one moved, which is a picture of nothing. What the reader gets instead is what the
 * numbers are — a busy series is visibly taller than a quiet one.
 *
 * Bars rather than a line, because a line implies the values between the samples were measured and
 * they were not: the monitor samples on a timer, and a bar says "this is what the interval was".
 *
 * Nothing is drawn for an empty history, and nothing pretends to be a chart of zeroes: the row
 * keeps its height so the panel does not jump when the first sample arrives.
 */
import { computed } from "vue";

export interface SparkSeries {
  readonly values: readonly number[];
  /** The bar's colour, as a Tailwind class; the panel decides what each series means. */
  readonly class: string;
}

const props = defineProps<{
  series: readonly SparkSeries[];
  /** What the bars mean, for a reader who cannot see them. */
  label: string;
  /** How tall the row is. The panel's charts are small; a page that is about them draws taller. */
  heightClass?: string;
}>();

/** The tallest value on screen. At least one, so an idle host draws a flat row rather than nothing. */
const peak = computed(() => Math.max(1, ...props.series.flatMap((entry) => entry.values)));

/** How many bars one row holds: the longest series, with the others padded by the template. */
const samples = computed(() => Math.max(0, ...props.series.map((entry) => entry.values.length)));

function height(value: number | undefined): string {
  const share = (value ?? 0) / peak.value;
  return `${Math.round(Math.min(Math.max(share, 0), 1) * 100)}%`;
}
</script>

<template>
  <div class="flex items-end gap-px" :class="props.heightClass ?? 'h-10'" role="img" :aria-label="props.label">
    <div v-for="index in samples" :key="index" class="flex h-full min-w-0 flex-1 items-end gap-px">
      <span
        v-for="(entry, seriesIndex) in props.series"
        :key="seriesIndex"
        class="min-w-0 flex-1 rounded-t-[1px]"
        :class="entry.class"
        :style="{ height: height(entry.values[index - 1]) }"
      />
    </div>
  </div>
</template>
