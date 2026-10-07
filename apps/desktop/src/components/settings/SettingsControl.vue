<script setup lang="ts">
/**
 * The control a settings row carries, chosen by what the row is.
 *
 * It exists so that the page draws a row once: search results and the category body show the same
 * rows, and a second copy of this switch would be a second place for the two to drift apart.
 *
 * What a row gets is decided by what it is, not by where it is drawn: a switch for something that
 * is on or off, three buttons for a mode with three answers, a dropdown for a list of values, and a
 * font picker for a family. The table says which, and this is the only place that knows how to draw
 * one — which is why the width is fixed on the dropdowns: they line up down the right edge of the
 * page, and two rows with different option lengths would otherwise put their arrows in two places.
 */
import type { Component } from "vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { Sun, Moon, SunMoon } from "@lucide/vue";

import { FONT_LABEL_KEYS, type SettingsChoiceIcon, type SettingsRowDefinition } from "@/components/settings/rows";
import FontSelect from "@/components/ui/FontSelect.vue";
import { cn } from "@/lib/utils";
import Select from "@/components/ui/Select.vue";
import Switch from "@/components/ui/Switch.vue";
import {
  fontOptionsFor,
  loadSystemFontNames,
  normalizeCustomFontFamilyInput,
  readableFontFamily,
} from "@/lib/settings/fonts";

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

// -- The font picker ---------------------------------------------------------

/**
 * This machine's families, read the first time a font picker is opened.
 *
 * Not when the page is drawn: the read is several hundred font files, and a page of settings that
 * spends that on opening is a page that looks slow for the sake of a list nobody has asked for yet.
 */
const systemFonts = ref<readonly string[]>([]);
const reading = ref(false);

async function readFonts(): Promise<void> {
  if (systemFonts.value.length > 0 || reading.value) {
    return;
  }
  reading.value = true;
  systemFonts.value = await loadSystemFontNames();
  reading.value = false;
}

const fontOptions = computed(() =>
  props.row.control.kind === "font"
    ? fontOptionsFor(props.row.control.family, systemFonts.value, String(props.value))
    : [],
);

/** What a stack is called: a name for the ones that have one, the system's spelling for the rest. */
function fontName(stack: string): string {
  const key = FONT_LABEL_KEYS[stack];
  return key ? t(key) : readableFontFamily(stack);
}

// -- The rest ----------------------------------------------------------------

/**
 * Which glyph each name stands for.
 *
 * The table says `sun`, and this is what a sun is — a component cannot be written into a table of
 * data without turning the table into code, and the set is closed so a name that has no glyph here
 * is a compile error rather than an empty button.
 */
const CHOICE_ICONS: Record<SettingsChoiceIcon, Component> = {
  sun: Sun,
  moon: Moon,
  "sun-moon": SunMoon,
};

/** A set of choices, ready to draw: what each is worth, what it says, and its glyph. */
const choices = computed(() =>
  props.row.control.kind === "choice"
    ? props.row.control.options.map((option) => ({
        value: option.value,
        label: t(option.labelKey),
        icon: option.icon ? CHOICE_ICONS[option.icon] : undefined,
        radius: option.radius,
      }))
    : [],
);

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

  <!--
    The three modes as three buttons, one of them lit. A radio group rather than a set of pressed
    buttons, because that is what it is: one of three, always one — and a reader who cannot see the
    lit border hears which one it is.
  -->
  <div
    v-else-if="props.row.control.kind === 'choice'"
    role="radiogroup"
    :aria-label="props.label"
    class="flex flex-wrap gap-2"
  >
    <button
      v-for="option in choices"
      :key="option.value"
      type="button"
      role="radio"
      :aria-checked="String(props.value) === option.value"
      :class="
        cn(
          'flex h-8 items-center gap-1.5 rounded-md border px-3 text-sm transition-colors',
          String(props.value) === option.value
            ? 'border-primary/40 bg-primary/10 text-primary ring-1 ring-primary/30'
            : 'border-border text-foreground hover:bg-accent',
        )
      "
      :style="option.radius ? { borderRadius: option.radius } : undefined"
      @click="emit('choose', option.value)"
    >
      <component :is="option.icon" v-if="option.icon" :size="15" />
      {{ option.label }}
    </button>
  </div>

  <FontSelect
    v-else-if="props.row.control.kind === 'font'"
    :model-value="String(props.value)"
    :options="fontOptions"
    :name="fontName"
    :aria-label="props.label"
    :search-placeholder="t('settings.font.search')"
    :loading="reading"
    :loading-text="t('settings.font.loading')"
    :empty-text="t('settings.font.empty')"
    :use-typed-text="(value: string) => t('settings.font.use', { font: value })"
    allow-custom
    :normalize-custom="normalizeCustomFontFamilyInput"
    :trigger-class="props.stretch ? 'h-9 w-full' : 'h-9 w-44'"
    @update:model-value="(value: string) => emit('choose', value)"
    @open="(open: boolean) => open && readFonts()"
    @preview="(value: string) => emit('preview', value)"
    @end-preview="emit('endPreview')"
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
