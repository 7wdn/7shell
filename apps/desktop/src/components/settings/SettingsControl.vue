<script setup lang="ts">
/**
 * The control a settings row carries, chosen by what the row is.
 *
 * It exists so that the page draws a row once: search results and the category body show the same
 * rows, and a second copy of this switch would be a second place for the two to drift apart.
 *
 * The width on the dropdown is not decoration: the controls line up down the right edge of the
 * page, so two rows with different option lengths would otherwise put their arrows in two places.
 */
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import type { SettingsRowDefinition } from "@/components/settings/rows";
import Select from "@/components/ui/Select.vue";
import Switch from "@/components/ui/Switch.vue";

const props = defineProps<{
  row: SettingsRowDefinition;
  /** What the field is now — a string for a dropdown, a boolean for a switch. */
  value: string | boolean;
  /** The control's accessible name, which is the row's own title. */
  label: string;
  /**
   * Whether the control fills the column it is in.
   *
   * A dropdown in a row is one width for the whole page, so that the arrows of two rows line up
   * down the right edge; a dropdown in a field of a grid has the column to fill instead, and a
   * fixed width there would leave three controls of the same size floating in three columns of
   * different ones.
   */
  stretch?: boolean;
}>();

const emit = defineEmits<{
  choose: [value: string | boolean];
  /** An option is under the pointer: what the setting would look like if it were taken. */
  preview: [value: string];
  /** The pointer is off the list: show what was actually chosen again. */
  endPreview: [];
}>();

const { t } = useI18n();

const options = computed(() =>
  props.row.control.kind === "select"
    ? props.row.control.options.map((option) => ({
        value: option.value,
        // A label with something filled in — a percentage — arrives as a key and its values, so
        // that the number is not a sentence assembled in the code.
        label: option.params ? t(option.labelKey, option.params) : t(option.labelKey),
        swatch: option.swatch,
      }))
    : [],
);
</script>

<template>
  <Switch
    v-if="props.row.control.kind === 'switch'"
    :model-value="props.value === true"
    :aria-label="props.label"
    @update:model-value="(value: boolean) => emit('choose', value)"
  />

  <Select
    v-else
    :model-value="String(props.value)"
    :options="options"
    :aria-label="props.label"
    :trigger-class="props.stretch ? 'w-full' : 'w-44'"
    @update:model-value="(value: string) => emit('choose', value)"
    @option-hover="(value: string) => emit('preview', value)"
    @option-leave="emit('endPreview')"
  />
</template>
