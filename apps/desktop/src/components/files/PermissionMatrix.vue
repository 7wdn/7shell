<script setup lang="ts">
/**
 * The nine permission boxes, and the octal draft they edit.
 *
 * The draft is the source of truth: typing `0755` ticks the boxes, and ticking a box rewrites the
 * draft. Special bits are carried through the round trip even though this grid has no column for
 * them.
 */
import { useI18n } from "vue-i18n";

import { PERMISSION_COLUMNS, PERMISSION_ROLES, hasPermission, togglePermission } from "@/lib/files/mode";

const props = withDefaults(defineProps<{ modelValue: string; label: string; disabled?: boolean }>(), {
  disabled: false,
});
const emit = defineEmits<{ "update:modelValue": [value: string] }>();

const { t } = useI18n();

function changed(shift: number, bit: number, event: Event): void {
  const input = event.target;
  if (input instanceof HTMLInputElement) {
    emit("update:modelValue", togglePermission(props.modelValue, shift, bit, input.checked));
  }
}
</script>

<template>
  <div
    class="grid grid-cols-[5.5rem_repeat(3,minmax(0,1fr))] items-center gap-y-1 text-xs"
    role="group"
    :aria-label="props.label"
  >
    <span></span>
    <span v-for="column in PERMISSION_COLUMNS" :key="column.key" class="text-center text-muted-foreground">
      {{ t(`files.chmod.columns.${column.key}`) }}
    </span>
    <template v-for="role in PERMISSION_ROLES" :key="role.key">
      <span class="text-muted-foreground">{{ t(`files.chmod.roles.${role.key}`) }}</span>
      <label v-for="column in PERMISSION_COLUMNS" :key="column.key" class="grid place-items-center">
        <input
          type="checkbox"
          class="size-3.5 accent-primary"
          :disabled="props.disabled"
          :checked="hasPermission(props.modelValue, role.shift, column.bit)"
          :aria-label="`${t(`files.chmod.roles.${role.key}`)} ${t(`files.chmod.columns.${column.key}`)}`"
          @change="changed(role.shift, column.bit, $event)"
        />
      </label>
    </template>
  </div>
</template>
