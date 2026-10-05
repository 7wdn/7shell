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
}>();

const emit = defineEmits<{ choose: [value: string | boolean] }>();

const { t } = useI18n();

const options = computed(() =>
  props.row.control.kind === "select"
    ? props.row.control.options.map((option) => ({ value: option.value, label: t(option.labelKey) }))
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
    trigger-class="w-44"
    @update:model-value="(value: string) => emit('choose', value)"
  />
</template>
