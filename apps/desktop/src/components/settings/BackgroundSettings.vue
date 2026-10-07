<script setup lang="ts">
/**
 * The picture behind the window, as the block that chooses and shapes it.
 *
 * Five answers in one block because they are one thing: a picture, how it is fitted, how much of it
 * shows through and how far it is blurred. The four numbers are read and written through the store
 * (they are settings, and they take effect in the window as they are changed, like every other choice
 * on this page); the *file* goes through the shell, because opening a picker and copying a picture is
 * exactly the kind of work the interface does not do — it is handed the picture's name, never a path.
 *
 * The name is what the line under the buttons says, and it is the name the file had when it was chosen:
 * this installation's own copy is called something else, and nothing here knows or needs to.
 */
import { LoaderCircle, Trash2, Upload } from "@lucide/vue";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import Select from "@/components/ui/Select.vue";
import { useToast } from "@/composables/useToast";
import { describeShellError } from "@/i18n/errors";
import { backend } from "@/lib/backend/api";
import { asShellError } from "@/lib/backend/errors";
import { usePreferencesStore } from "@/stores/preferences";
import {
  BACKGROUND_BLUR_MAX,
  BACKGROUND_DISPLAY_MODES,
  BACKGROUND_TRANSPARENCY_MAX,
  type BackgroundDisplayMode,
} from "@/types/app";

const { t } = useI18n();
const preferences = usePreferencesStore();
const { toast } = useToast();

const working = ref(false);

/** The picture's name, or the line that says there is none. */
const name = computed(() => preferences.backgroundImage.fileName?.trim() ?? "");

/**
 * What the transparency slider shows.
 *
 * The number in the settings file *is* the transparency — the same number the label reads out — so
 * there is nothing to convert here; the conversion is the CSS side's, where the surfaces take what is
 * left of the colour.
 */
const transparency = computed(() => preferences.backgroundImage.transparency);

const modes = computed(() =>
  BACKGROUND_DISPLAY_MODES.map((mode) => ({
    value: mode,
    label: t(`settings.background.mode${mode[0]?.toUpperCase()}${mode.slice(1)}`),
  })),
);

/** Opens the system's picker, copies what comes back, and says what was copied. */
async function choose(): Promise<void> {
  working.value = true;
  try {
    const chosen = await backend.background.choose(t("settings.background.filterName"));
    if (chosen) {
      preferences.setBackgroundImageName(chosen);
    }
  } catch (error) {
    toast(describeShellError(translate, asShellError(error, "background_image_choose")).message, "failure");
  } finally {
    working.value = false;
  }
}

/**
 * Takes the picture away.
 *
 * The copy goes now rather than when the page is written, because the two are one act: the settings
 * file names a picture, and a name with no picture behind it is a window that draws nothing and says
 * it should draw something. A copy that will not go is said out loud instead of being forgotten.
 */
async function clear(): Promise<void> {
  working.value = true;
  try {
    await backend.background.clear();
    preferences.setBackgroundImageName(null);
  } catch (error) {
    toast(describeShellError(translate, asShellError(error, "background_image_clear")).message, "failure");
  } finally {
    working.value = false;
  }
}

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}
</script>

<template>
  <div class="min-w-0 space-y-3">
    <div class="flex min-w-0 flex-wrap items-center gap-2">
      <button
        type="button"
        class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
        :disabled="working"
        @click="choose"
      >
        <LoaderCircle v-if="working" :size="13" class="animate-spin" />
        <Upload v-else :size="13" />
        {{ t("settings.background.choose") }}
      </button>

      <button
        v-if="name"
        type="button"
        class="flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
        :disabled="working"
        @click="clear"
      >
        <Trash2 :size="13" />
        {{ t("settings.background.clear") }}
      </button>

      <span v-if="name" class="min-w-0 truncate text-xs text-muted-foreground" :title="name">{{ name }}</span>
      <span v-else class="text-xs text-muted-foreground">{{ t("settings.background.none") }}</span>
    </div>

    <div class="grid min-w-0 gap-3 sm:grid-cols-2">
      <label class="min-w-0 space-y-1.5">
        <span class="text-xs font-medium">{{ t("settings.background.displayMode") }}</span>
        <Select
          :model-value="preferences.backgroundImage.displayMode"
          :options="modes"
          :label="t('settings.background.displayMode')"
          @choose="(value: string) => preferences.setBackgroundDisplayMode(value as BackgroundDisplayMode)"
        />
      </label>

      <div class="hidden sm:block"></div>

      <label class="min-w-0 space-y-1.5">
        <span class="flex items-center justify-between gap-2 text-xs font-medium">
          {{ t("settings.background.transparency") }}
          <span class="font-normal text-muted-foreground">{{ transparency }}%</span>
        </span>
        <input
          type="range"
          class="w-full accent-primary"
          min="0"
          :max="BACKGROUND_TRANSPARENCY_MAX"
          step="1"
          :value="transparency"
          :aria-label="t('settings.background.transparency')"
          @input="preferences.setBackgroundTransparency(Number(($event.target as HTMLInputElement).value))"
        />
      </label>

      <label class="min-w-0 space-y-1.5">
        <span class="flex items-center justify-between gap-2 text-xs font-medium">
          {{ t("settings.background.blur") }}
          <span class="font-normal text-muted-foreground">{{ preferences.backgroundImage.blur }}px</span>
        </span>
        <input
          type="range"
          class="w-full accent-primary"
          min="0"
          :max="BACKGROUND_BLUR_MAX"
          step="1"
          :value="preferences.backgroundImage.blur"
          :aria-label="t('settings.background.blur')"
          @input="preferences.setBackgroundBlur(Number(($event.target as HTMLInputElement).value))"
        />
      </label>
    </div>
  </div>
</template>
