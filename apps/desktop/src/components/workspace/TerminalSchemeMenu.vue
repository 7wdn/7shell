<script setup lang="ts">
/**
 * The colour scheme, one click from the terminal it dresses.
 *
 * The settings page is where schemes are imported and arranged; this is where one is *taken*, which
 * is a different act and belongs beside the thing it changes. The list is the slot the terminal is
 * currently in — dark or light — and choosing from it writes the choice down at once: a quick
 * switch whose result vanished on restart would be a preview, not a switch.
 *
 * Writing goes through the ordinary preferences store, so the settings file and every open pane
 * agree afterwards; the settings page's own list reflects the change the next time it is looked at.
 */
import { Check, Palette, Search } from "@lucide/vue";
import { PopoverContent, PopoverPortal, PopoverRoot, PopoverTrigger } from "reka-ui";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { schemeTone, TERMINAL_COLOR_SCHEMES } from "@/lib/terminal/schemes";
import { usePreferencesStore } from "@/stores/preferences";

const { t } = useI18n();
const preferences = usePreferencesStore();

const open = ref(false);
const query = ref("");

const currentId = computed(() => {
  if (preferences.terminalSchemeSource !== "custom") {
    return null;
  }
  return preferences.resolvedTerminalTheme === "dark"
    ? preferences.terminalSchemeDark
    : preferences.terminalSchemeLight;
});

const schemes = computed(() => [...TERMINAL_COLOR_SCHEMES, ...preferences.terminalCustomSchemes]);

const visible = computed(() => {
  const needle = query.value.trim().toLowerCase();
  const tone = preferences.resolvedTerminalTheme;
  return schemes.value
    .filter((scheme) => schemeTone(scheme) === tone)
    .filter((scheme) => needle === "" || scheme.name.toLowerCase().includes(needle) || scheme.id.includes(needle))
    .sort((a, b) => a.name.localeCompare(b.name));
});

function setOpen(value: boolean): void {
  open.value = value;
  if (!value) {
    query.value = "";
  }
}

/** Takes a scheme for the half of the terminal that is on screen, and remembers it. */
async function take(id: string | null): Promise<void> {
  if (id === null) {
    preferences.setTerminalSchemeSource("host");
  } else {
    preferences.setTerminalSchemeSource("custom");
    if (preferences.resolvedTerminalTheme === "dark") {
      preferences.setTerminalSchemeDark(id);
    } else {
      preferences.setTerminalSchemeLight(id);
    }
  }
  open.value = false;
  query.value = "";
  await preferences.commit();
}
</script>

<template>
  <PopoverRoot :open="open" @update:open="setOpen">
    <PopoverTrigger as-child>
      <button
        type="button"
        class="flex shrink-0 items-center gap-1 rounded-md px-2 py-0.5 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        :title="t('terminal.scheme.title')"
        :aria-label="t('terminal.scheme.title')"
        :aria-expanded="open"
      >
        <Palette :size="11" />
        {{ t("terminal.scheme.label") }}
      </button>
    </PopoverTrigger>

    <PopoverPortal>
      <PopoverContent
        side="top"
        align="end"
        :side-offset="6"
        :collision-padding="8"
        class="z-50 flex max-h-[min(22rem,50vh)] w-72 flex-col rounded-lg border border-border bg-card p-1.5 shadow-xl"
      >
        <label class="relative m-1 block">
          <Search :size="12" class="absolute top-1/2 left-2 -translate-y-1/2 text-muted-foreground" />
          <input
            v-model="query"
            type="search"
            spellcheck="false"
            class="w-full rounded-md border border-border bg-background py-1 pr-2 pl-6 text-xs focus:border-primary focus:outline-none"
            :placeholder="t('terminal.scheme.search')"
            :aria-label="t('terminal.scheme.search')"
          />
        </label>

        <div class="min-h-0 flex-1 overflow-y-auto scroll-thin" role="listbox">
          <button
            type="button"
            role="option"
            :aria-selected="preferences.terminalSchemeSource === 'host'"
            class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent"
            :class="preferences.terminalSchemeSource === 'host' ? 'bg-primary/10 text-primary' : ''"
            @click="take(null)"
          >
            <span class="min-w-0 flex-1 truncate">{{ t("settings.schemes.followPalette") }}</span>
            <Check v-if="preferences.terminalSchemeSource === 'host'" :size="12" class="shrink-0" />
          </button>

          <button
            v-for="scheme in visible"
            :key="scheme.id"
            type="button"
            role="option"
            :aria-selected="currentId === scheme.id"
            class="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-left text-xs transition-colors hover:bg-accent"
            :class="currentId === scheme.id ? 'bg-primary/10 text-primary' : ''"
            @click="take(scheme.id)"
          >
            <span class="flex shrink-0 gap-px" aria-hidden="true">
              <i
                v-for="(colour, index) in scheme.colors.slice(0, 12)"
                :key="index"
                class="block h-3 w-1 rounded-[1px]"
                :style="{ background: colour }"
              />
            </span>
            <span class="min-w-0 flex-1 truncate">{{ scheme.name }}</span>
            <Check v-if="currentId === scheme.id" :size="12" class="shrink-0" />
          </button>

          <p v-if="visible.length === 0" class="px-2 py-6 text-center text-[11px] text-muted-foreground">
            {{ t("settings.schemes.empty") }}
          </p>
        </div>
      </PopoverContent>
    </PopoverPortal>
  </PopoverRoot>
</template>
