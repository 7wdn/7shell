<script setup lang="ts">
/**
 * The one-line form that names a new group: under the list's header for one at the root, and under
 * a group's own heading for one made inside it.
 *
 * Renaming is not here: a name that already exists is changed where it is written — the label in
 * the row becomes a field — because that is the thing the typist is looking at. This form exists
 * for the case with nothing to look at yet.
 *
 * The keyboard is Enter to make it and the cross to throw it away, and there is deliberately no
 * commit-on-blur: the cross sits *inside* this form, and clicking it blurs the field first — a blur
 * that saved would turn "cancel" into "save and close".
 */
import { X } from "@lucide/vue";
import { ref, type Component } from "vue";
import { useI18n } from "vue-i18n";

defineProps<{
  readonly modelValue: string;
  /** The icon in front: the folder for a group being made, the pencil for one being renamed. */
  readonly icon: Component;
  /** What the submit button says. */
  readonly submitLabel: string;
  readonly busy: boolean;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
  submit: [];
  cancel: [];
}>();

const { t } = useI18n();
const field = ref<HTMLInputElement | null>(null);

/** Puts the cursor in the field, which is what a form that was just opened owes the person. */
function focus(): void {
  field.value?.focus();
  field.value?.select();
}

defineExpose({ focus });
</script>

<template>
  <form
    class="mt-1 mb-1.5 flex items-center gap-1.5 rounded-md border border-border bg-background px-2 py-1.5"
    @submit.prevent="emit('submit')"
  >
    <component :is="icon" :size="12" class="shrink-0 text-muted-foreground" />
    <input
      ref="field"
      :value="modelValue"
      type="text"
      class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-[11px] focus:outline-none"
      :aria-label="t('sessions.list.groupName')"
      :placeholder="t('sessions.list.groupName')"
      @input="emit('update:modelValue', ($event.target as HTMLInputElement).value)"
    />
    <button
      type="submit"
      class="shrink-0 rounded-md bg-primary px-2 py-1 text-[11px] text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
      :disabled="busy || modelValue.trim() === ''"
    >
      {{ submitLabel }}
    </button>
    <button
      type="button"
      class="grid size-5 shrink-0 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      :title="t('sessions.list.cancel')"
      :aria-label="t('sessions.list.cancel')"
      @click="emit('cancel')"
    >
      <X :size="11" />
    </button>
  </form>
</template>
