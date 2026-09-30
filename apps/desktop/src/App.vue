<script setup lang="ts">
import { storeToRefs } from "pinia";
import { computed, onMounted } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { SUPPORTED_LOCALES, type SupportedLocale } from "@/i18n";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";
import { CAPABILITIES, protocolSupports } from "@/types/generated/protocols";

const { t } = useI18n();
const app = useAppStore();
const preferences = usePreferencesStore();

const { info, status, failure, platform } = storeToRefs(app);
const { locale, theme } = storeToRefs(preferences);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const describedFailure = computed(() => (failure.value ? describeShellError(translate, failure.value) : null));

// The capability list comes from the descriptors, not from a hand-written array: the
// interface can only offer what a descriptor claims, and Rust reads the same source.
const capabilities = computed(() =>
  CAPABILITIES.map((capability) => ({
    key: capability.key,
    label: t(capability.labelKey),
    supported: protocolSupports("ssh", capability.key),
  })),
);

const statusLabel = computed(() => {
  if (status.value === "loading") {
    return t("common.status.loading");
  }
  return status.value === "failed" ? t("common.status.failed") : t("common.status.ready");
});

const statusClass = computed(() => {
  if (status.value === "failed") {
    return "text-destructive";
  }
  return status.value === "ready" ? "text-success" : "text-muted-foreground";
});

function onLocaleChange(event: Event): void {
  const target = event.target as HTMLSelectElement | null;
  const next = target?.value ?? "";
  if ((SUPPORTED_LOCALES as readonly string[]).includes(next)) {
    preferences.setLocale(next as SupportedLocale);
  }
}

onMounted(() => {
  preferences.initialize();
  void app.load();
});
</script>

<template>
  <div class="flex min-h-full flex-col">
    <header class="flex flex-wrap items-center justify-between gap-4 border-b border-border px-5 py-3">
      <div class="flex items-baseline gap-3">
        <h1 class="text-lg font-semibold tracking-tight">{{ t("app.title") }}</h1>
        <p class="text-sm text-muted-foreground">{{ t("app.subtitle") }}</p>
      </div>

      <div class="flex items-center gap-4 text-sm">
        <label class="flex items-center gap-2">
          <span class="text-muted-foreground">{{ t("common.preference.language") }}</span>
          <select
            :value="locale"
            class="rounded-md border border-input bg-card px-2 py-1 text-foreground"
            @change="onLocaleChange"
          >
            <option v-for="supported in SUPPORTED_LOCALES" :key="supported" :value="supported">
              {{ supported }}
            </option>
          </select>
        </label>

        <button
          type="button"
          class="rounded-md border border-border px-3 py-1 transition-colors hover:bg-accent"
          @click="preferences.toggleTheme()"
        >
          {{ theme === "dark" ? t("common.theme.light") : t("common.theme.dark") }}
        </button>
      </div>
    </header>

    <main class="grid flex-1 content-start gap-4 p-5 lg:grid-cols-2">
      <section class="rounded-lg border border-border bg-card p-4">
        <div class="mb-3 flex items-center justify-between">
          <h2 class="text-sm font-medium">{{ t("app.section.backend") }}</h2>
          <span class="text-xs" :class="statusClass">{{ statusLabel }}</span>
        </div>

        <dl v-if="info" class="grid grid-cols-[auto_1fr] gap-x-6 gap-y-2 text-sm">
          <dt class="text-muted-foreground">{{ t("app.field.version") }}</dt>
          <dd class="font-mono">{{ info.version }}</dd>

          <dt class="text-muted-foreground">{{ t("app.field.platform") }}</dt>
          <dd class="font-mono">{{ platform }}</dd>

          <dt class="text-muted-foreground">{{ t("app.field.protocols") }}</dt>
          <dd class="font-mono">{{ info.protocolCount }}</dd>

          <dt class="text-muted-foreground">{{ t("app.field.descriptorSchema") }}</dt>
          <dd class="font-mono">{{ info.descriptorSchemaVersion }}</dd>
        </dl>

        <div v-if="describedFailure" class="mt-3 rounded-md border border-destructive/40 bg-destructive/10 p-3 text-sm">
          <p>{{ describedFailure.message }}</p>
          <p class="mt-1 font-mono text-xs text-muted-foreground">{{ describedFailure.code }}</p>
          <p v-if="describedFailure.detail" class="mt-1 text-xs text-muted-foreground">{{ describedFailure.detail }}</p>
        </div>

        <button
          type="button"
          class="mt-4 rounded-md border border-border px-3 py-1 text-sm transition-colors hover:bg-accent"
          @click="app.load()"
        >
          {{ status === "failed" ? t("common.action.retry") : t("common.action.reload") }}
        </button>
      </section>

      <section class="rounded-lg border border-border bg-card p-4">
        <h2 class="mb-3 text-sm font-medium">
          {{ t("app.section.capabilities", { protocol: t("protocol.ssh.label") }) }}
        </h2>

        <ul class="space-y-1 text-sm">
          <li v-for="capability in capabilities" :key="capability.key" class="flex items-center justify-between gap-4">
            <span :class="capability.supported ? '' : 'text-muted-foreground'">{{ capability.label }}</span>
            <span class="text-xs" :class="capability.supported ? 'text-success' : 'text-muted-foreground'">
              {{ capability.supported ? t("app.state.supported") : t("app.state.planned") }}
            </span>
          </li>
        </ul>
      </section>
    </main>

    <footer class="border-t border-border px-5 py-3 text-xs text-muted-foreground">
      {{ t("common.notice.early") }}
    </footer>
  </div>
</template>
