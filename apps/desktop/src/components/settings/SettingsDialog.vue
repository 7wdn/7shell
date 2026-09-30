<script setup lang="ts">
/**
 * Settings.
 *
 * Appearance, language and what this build is. Changes apply as they are made and are written
 * afterwards, so the dialog never has a "save" button that can be forgotten — and a write that
 * fails is shown here rather than swallowed.
 */
import { Monitor, Moon, Sun, X } from "@lucide/vue";
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import { computed, markRaw, type Component } from "vue";
import { useI18n } from "vue-i18n";

import { SUPPORTED_LOCALES, type SupportedLocale } from "@/i18n";
import { describeShellError } from "@/i18n/errors";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import type { ThemeMode } from "@/types/app";

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const app = useAppStore();
const preferences = usePreferencesStore();

const THEMES: readonly { mode: ThemeMode; labelKey: string; icon: Component }[] = [
  { mode: "system", labelKey: "settings.theme.system", icon: markRaw(Monitor) },
  { mode: "dark", labelKey: "settings.theme.dark", icon: markRaw(Moon) },
  { mode: "light", labelKey: "settings.theme.light", icon: markRaw(Sun) },
];

const LANGUAGE_LABELS: Record<SupportedLocale, string> = { en: "English", "zh-CN": "简体中文" };

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const problem = computed(() => (preferences.problem ? describeShellError(translate, preferences.problem) : null));
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <header class="flex items-center justify-between border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">{{ t("settings.title") }}</DialogTitle>
          <DialogDescription class="sr-only">{{ t("settings.description") }}</DialogDescription>
          <DialogClose
            class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('common.action.close')"
          >
            <X :size="15" />
          </DialogClose>
        </header>

        <div class="space-y-5 px-4 py-4">
          <section>
            <h3 class="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {{ t("settings.appearance") }}
            </h3>
            <div class="flex gap-1 rounded-md border border-border bg-background p-0.5">
              <button
                v-for="theme in THEMES"
                :key="theme.mode"
                type="button"
                class="flex flex-1 items-center justify-center gap-1.5 rounded-sm px-2 py-1.5 text-xs transition-colors"
                :class="
                  preferences.themeMode === theme.mode
                    ? 'bg-accent text-accent-foreground'
                    : 'text-muted-foreground hover:bg-accent/50'
                "
                :aria-pressed="preferences.themeMode === theme.mode"
                @click="preferences.setThemeMode(theme.mode)"
              >
                <component :is="theme.icon" :size="13" />
                {{ t(theme.labelKey) }}
              </button>
            </div>
            <p class="mt-2 text-xs text-muted-foreground">{{ t("settings.appearanceHint") }}</p>
          </section>

          <section>
            <h3 class="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {{ t("settings.language") }}
            </h3>
            <div class="flex gap-1 rounded-md border border-border bg-background p-0.5">
              <button
                v-for="supported in SUPPORTED_LOCALES"
                :key="supported"
                type="button"
                class="flex-1 rounded-sm px-2 py-1.5 text-xs transition-colors"
                :class="
                  preferences.locale === supported
                    ? 'bg-accent text-accent-foreground'
                    : 'text-muted-foreground hover:bg-accent/50'
                "
                :aria-pressed="preferences.locale === supported"
                @click="preferences.setLocale(supported)"
              >
                {{ LANGUAGE_LABELS[supported] }}
              </button>
            </div>
          </section>

          <section>
            <h3 class="mb-2 text-xs font-medium tracking-wide text-muted-foreground uppercase">
              {{ t("settings.about") }}
            </h3>
            <dl v-if="app.info" class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1.5 text-xs">
              <dt class="text-muted-foreground">{{ t("home.about.version") }}</dt>
              <dd class="font-mono">{{ app.info.version }}</dd>
              <dt class="text-muted-foreground">{{ t("home.about.platform") }}</dt>
              <dd class="font-mono">{{ app.info.os }} {{ app.info.arch }}</dd>
              <dt class="text-muted-foreground">{{ t("home.about.descriptorSchema") }}</dt>
              <dd class="font-mono">{{ app.info.descriptorSchemaVersion }}</dd>
            </dl>
            <p v-else class="text-xs text-muted-foreground">{{ t("common.status.loading") }}</p>
          </section>

          <p v-if="problem" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
            <span>{{ problem.message }}</span>
            <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
          </p>
        </div>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
