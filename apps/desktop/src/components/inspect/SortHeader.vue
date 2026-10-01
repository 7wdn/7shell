<script setup lang="ts">
/**
 * A column heading that sorts, in the one shape both tables use.
 *
 * It exists because two tables sorting the same way in two different ways is what happened: one
 * drew an arrow, the other a chevron, and neither said what pressing it again would do. A heading
 * that sorts is a control like any other, and a control that looks different on the next page is a
 * control somebody has to learn twice.
 *
 * The icons say what the table is doing rather than what pressing would do — up or down for the
 * column that is ordered, and the two-headed one for the columns that are not, which is the shape
 * every table in this application uses and the one people already read.
 */
import { ChevronDown, ChevronUp, ChevronsUpDown } from "@lucide/vue";

const props = defineProps<{
  label: string;
  /** Whether this is the column the table is ordered by. */
  sorted: boolean;
  /** Which way, when it is. */
  descending: boolean;
  /** What this column is and what pressing it does, for a reader who cannot see the table. */
  hint: string;
}>();

defineEmits<{ sort: [] }>();
</script>

<template>
  <button
    type="button"
    class="inline-flex items-center gap-1 rounded-sm px-1 transition-colors hover:bg-accent"
    :class="props.sorted ? 'text-foreground' : 'text-muted-foreground'"
    :title="props.hint"
    :aria-label="props.hint"
    @click="$emit('sort')"
  >
    {{ props.label }}
    <ChevronUp v-if="props.sorted && !props.descending" :size="11" class="shrink-0" />
    <ChevronDown v-else-if="props.sorted" :size="11" class="shrink-0" />
    <ChevronsUpDown v-else :size="11" class="shrink-0 opacity-40" />
  </button>
</template>
