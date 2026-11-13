<script setup lang="ts">
/**
 * One of the terminal's two scheme slots, and the list it is chosen from.
 *
 * A dialog rather than a dropdown: the catalogue has nearly two hundred entries and each one is
 * worth a strip of its own colours, which is the only way to tell two dark schemes apart at a
 * glance. The slot itself shows the chosen name and its colours; the list adds a search box and a
 * light/dark filter, because scrolling two hundred names to find one is not a choice.
 *
 * `null` is a real answer — the interface palette — and it is the first row rather than a separate
 * switch: "no scheme" is what this slot means when nothing has been chosen.
 */
import { ChevronDown, Check, Search, X } from "@lucide/vue";
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";

import { schemeTone, type TerminalColorScheme } from "@/lib/terminal/schemes";

const props = defineProps<{
  modelValue: string | null;
  /** What the slot is for, drawn above the trigger and as the dialog's title. */
  title: string;
  /** Which half of the theme the slot belongs to, used by the dialog's description. */
  tone: "dark" | "light";
  /** The catalogue to choose from: the shipped schemes and whatever was imported. */
  schemes: readonly TerminalColorScheme[];
}>();

const emit = defineEmits<{ "update:modelValue": [value: string | null] }>();

const { t } = useI18n();

const open = ref(false);
const query = ref("");
const tone = ref<"all" | "dark" | "light">("all");

const current = computed(() => props.schemes.find((scheme) => scheme.id === props.modelValue));

const visible = computed(() => {
  const needle = query.value.trim().toLowerCase();
  return props.schemes
    .filter((scheme) => {
      if (tone.value !== "all" && schemeTone(scheme) !== tone.value) {
        return false;
      }
      return needle === "" || scheme.name.toLowerCase().includes(needle) || scheme.id.includes(needle);
    })
    .sort((a, b) => a.name.localeCompare(b.name));
});

function choose(id: string | null): void {
  emit("update:modelValue", id);
  open.value = false;
}

function swatches(colors: readonly string[]): readonly string[] {
  return colors.slice(0, 16);
}

function show(): void {
  query.value = "";
  tone.value = "all";
  open.value = true;
}
</script>

<template>
  <div class="min-w-0 space-y-1.5">
    <span class="block text-xs font-medium text-muted-foreground">{{ props.title }}</span>
    <button
      type="button"
      data-scheme-slot
      class="flex w-full min-w-0 items-center gap-2 rounded-md border border-border bg-background px-2.5 py-1.5 text-left text-sm transition-colors hover:bg-accent focus-visible:border-primary focus-visible:outline-none"
      :aria-label="props.title"
      @click="show()"
    >
      <span v-if="current" class="flex shrink-0 gap-px" aria-hidden="true">
        <i
          v-for="(colour, index) in swatches(current.colors)"
          :key="index"
          class="block h-3.5 w-1.5 rounded-[1px]"
          :style="{ background: colour }"
        />
      </span>
      <span class="min-w-0 flex-1 truncate" :class="current ? '' : 'text-muted-foreground'">
        {{ current?.name ?? t("settings.schemes.followPalette") }}
      </span>
      <ChevronDown :size="14" class="shrink-0 text-muted-foreground" />
    </button>

    <DialogRoot :open="open" @update:open="(value: boolean) => (open = value)">
      <DialogPortal>
        <DialogOverlay class="fixed inset-0 z-50 bg-overlay" />
        <DialogContent
          class="fixed top-1/2 left-1/2 z-50 flex max-h-[min(42rem,calc(100vh-3rem))] w-[min(42rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl focus:outline-none"
        >
          <header class="flex items-center justify-between border-b border-border px-4 py-3">
            <DialogTitle class="text-sm font-medium">{{ props.title }}</DialogTitle>
            <DialogDescription class="sr-only">
              {{ t(`settings.schemes.description.${props.tone}`) }}
            </DialogDescription>
            <DialogClose
              class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              :aria-label="t('settings.schemes.close')"
            >
              <X :size="15" />
            </DialogClose>
          </header>

          <div class="flex flex-wrap items-center gap-2 border-b border-border px-4 py-2.5">
            <label class="relative min-w-0 flex-1">
              <Search :size="13" class="absolute top-1/2 left-2 -translate-y-1/2 text-muted-foreground" />
              <input
                v-model="query"
                type="search"
                spellcheck="false"
                class="w-full rounded-md border border-border bg-background py-1.5 pr-2 pl-7 text-sm focus:border-primary focus:outline-none"
                :placeholder="t('settings.schemes.search')"
                :aria-label="t('settings.schemes.search')"
              />
            </label>
            <div class="flex shrink-0 gap-1" role="group" :aria-label="t('settings.schemes.tone')">
              <button
                v-for="option in ['all', 'dark', 'light'] as const"
                :key="option"
                type="button"
                class="rounded-md border px-2.5 py-1 text-xs transition-colors"
                :class="
                  tone === option
                    ? 'border-primary/40 bg-primary/10 text-primary'
                    : 'border-border text-muted-foreground hover:bg-accent hover:text-foreground'
                "
                @click="tone = option"
              >
                {{
                  t(
                    option === "all"
                      ? "settings.schemes.toneAll"
                      : option === "dark"
                        ? "settings.schemes.toneDark"
                        : "settings.schemes.toneLight",
                  )
                }}
              </button>
            </div>
          </div>

          <div class="min-h-0 flex-1 overflow-y-auto p-2 scroll-thin" role="listbox">
            <button
              type="button"
              role="option"
              :aria-selected="props.modelValue === null"
              class="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors hover:bg-accent"
              :class="props.modelValue === null ? 'bg-primary/10 text-primary' : ''"
              @click="choose(null)"
            >
              <span class="min-w-0 flex-1 truncate">{{ t("settings.schemes.followPalette") }}</span>
              <Check v-if="props.modelValue === null" :size="14" class="shrink-0" />
            </button>

            <button
              v-for="scheme in visible"
              :key="scheme.id"
              type="button"
              role="option"
              :aria-selected="props.modelValue === scheme.id"
              class="flex w-full items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors hover:bg-accent"
              :class="props.modelValue === scheme.id ? 'bg-primary/10 text-primary' : ''"
              @click="choose(scheme.id)"
            >
              <span class="flex shrink-0 gap-px" aria-hidden="true">
                <i
                  v-for="(colour, index) in swatches(scheme.colors)"
                  :key="index"
                  class="block h-3.5 w-1.5 rounded-[1px]"
                  :style="{ background: colour }"
                />
              </span>
              <span class="min-w-0 flex-1 truncate">{{ scheme.name }}</span>
              <Check v-if="props.modelValue === scheme.id" :size="14" class="shrink-0" />
            </button>

            <p v-if="visible.length === 0" class="px-2.5 py-8 text-center text-xs text-muted-foreground">
              {{ t("settings.schemes.empty") }}
            </p>
          </div>
        </DialogContent>
      </DialogPortal>
    </DialogRoot>
  </div>
</template>
