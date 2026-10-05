<script setup lang="ts">
/**
 * The switch, for a setting that is either on or off.
 *
 * A switch rather than two words in a segmented control: "on" and "off" are not two choices of
 * equal weight, they are one state and its absence, and the control should say which one it is at a
 * glance. It is not the platform's own switch either — same reason as the dropdown beside it in the
 * settings rows: what the platform draws in a WebView comes from the system's palette rather than
 * from this window's.
 *
 * The accessible name is not written here. A switch draws no text of its own — the row's title is
 * what it is about — so the caller passes it, and what a screen reader announces comes from
 * `aria-checked`, which reka-ui keeps in step with the value.
 */
import { SwitchRoot, SwitchThumb } from "reka-ui";
import { computed } from "vue";

import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    modelValue: boolean;
    disabled?: boolean;
    /** The control's name: the row's own title, since a switch draws no text of its own. */
    ariaLabel?: string;
    id?: string;
    /** Escape hatch for a caller whose switch is not the plain one. */
    class?: string;
  }>(),
  { disabled: false, ariaLabel: undefined, id: undefined, class: undefined },
);

defineEmits<{ "update:modelValue": [value: boolean] }>();

const rootClass = computed(() =>
  cn(
    "inline-flex shrink-0 cursor-pointer items-center rounded-full border border-transparent bg-input transition-colors",
    "h-[18px] w-8",
    "data-[state=checked]:bg-primary",
    "focus-visible:ring-1 focus-visible:ring-ring focus-visible:outline-none",
    "disabled:cursor-not-allowed disabled:opacity-60",
    props.class,
  ),
);

const thumbClass =
  "pointer-events-none block size-3.5 rounded-full bg-background shadow-sm transition-transform data-[state=checked]:translate-x-4";
</script>

<template>
  <SwitchRoot
    :id="props.id"
    :model-value="props.modelValue"
    :disabled="props.disabled"
    :aria-label="props.ariaLabel"
    :class="rootClass"
    @update:model-value="(value: boolean) => $emit('update:modelValue', value)"
  >
    <SwitchThumb :class="thumbClass" />
  </SwitchRoot>
</template>
