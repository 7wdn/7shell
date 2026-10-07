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
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { FONT_LABEL_KEYS, type SettingsRowDefinition } from "@/components/settings/rows";
import FontSelect from "@/components/ui/FontSelect.vue";
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
