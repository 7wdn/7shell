<script setup lang="ts">
/**
 * The update channel: what is published, and installing it.
 *
 * It lives on the About page because that is where the version already is, and an update is a fact
 * about the version. Everything it draws comes from one answer — the store's — so the line saying
 * "nothing newer is published" and the line saying "0.1.1 is published" can never both be on
 * screen, which is the failure mode of a page that keeps its own copy of an answer.
 *
 * The install button is deliberately not a silent one: it starts a download of tens of megabytes,
 * and the progress line under it is the difference between "this is working" and "this did nothing".
 * What is *not* drawn is a success state, because there is nowhere to draw it: the application is
 * replaced and restarted by the time the call could return.
 */
import { Download, LoaderCircle, RefreshCw } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import Switch from "@/components/ui/Switch.vue";
import { describeShellError } from "@/i18n/errors";
import { usePreferencesStore } from "@/stores/preferences";
import { useUpdatesStore } from "@/stores/updates";

const { t } = useI18n();
const updates = useUpdatesStore();
const preferences = usePreferencesStore();

const release = computed(() => updates.release);

const buttonLabel = computed(() => {
  if (updates.asking) {
    return t("settings.update.checking");
  }
  return updates.answer ? t("settings.update.checkAgain") : t("settings.update.check");
});

/** The answer in one line, when the answer is not a release to install. */
const summary = computed(() => {
  const answer = updates.answer;
  if (!answer) {
    return null;
  }
  if (answer.kind === "upToDate") {
    return t("settings.update.upToDate", { version: answer.currentVersion });
  }
  if (answer.kind === "notInstalled") {
    return t("settings.update.notInstalled");
  }
  return null;
});

const problem = computed(() => (updates.failure ? describeShellError(translate, updates.failure) : null));

/** What the progress line says while an install runs, in the units a download is talked about. */
const progressLine = computed(() => {
  const progress = updates.progress;
  // Nothing until the shell says how far it has got: the button's own spinner is the answer to
  // "did the click take", and a line inventing a number would be a claim about a download nobody
  // has measured yet.
  if (!progress) {
    return null;
  }
  if (progress.kind === "installing") {
    return t("settings.update.installing");
  }
  const done = bytes(progress.downloaded);
  return progress.total === undefined
    ? t("settings.update.downloadingUnknown", { done })
    : t("settings.update.downloading", { done, total: bytes(progress.total) });
});

function bytes(count: number): string {
  if (count < 1024 * 1024) {
    return `${Math.round(count / 1024)} kB`;
  }
  return `${(count / (1024 * 1024)).toFixed(1)} MB`;
}

/** A published date somebody can place on a calendar, from seconds since the epoch. */
function publishedAt(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString();
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
        :disabled="updates.asking || updates.installing"
        @click="updates.check()"
      >
        <LoaderCircle v-if="updates.asking" :size="13" class="animate-spin" />
        <RefreshCw v-else :size="13" />
        {{ buttonLabel }}
      </button>
    </div>

    <p v-if="summary" class="text-xs text-muted-foreground">{{ summary }}</p>

    <!--
      A release to install, with what it says about itself. The notes are published prose and are
      drawn as prose: they are the one place in this window where somebody else's words appear, and
      turning them into markup would be this build deciding how somebody else's sentences look.
    -->
    <div v-if="release" class="min-w-0 space-y-3 rounded-md border border-border bg-muted/20 px-3 py-2.5">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div class="min-w-0">
          <p class="text-sm font-medium">{{ t("settings.update.available", { version: release.version }) }}</p>
          <p v-if="release.publishedAt" class="text-[11px] text-muted-foreground">
            {{ t("settings.update.published", { when: publishedAt(release.publishedAt) }) }}
          </p>
        </div>

        <button
          type="button"
          class="flex items-center gap-1.5 rounded-md bg-primary px-2.5 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="updates.installing"
          @click="updates.install()"
        >
          <LoaderCircle v-if="updates.installing" :size="13" class="animate-spin" />
          <Download v-else :size="13" />
          {{ t("settings.update.install") }}
        </button>
      </div>

      <div v-if="release.notes" class="min-w-0 space-y-1">
        <h4 class="text-xs font-medium text-muted-foreground">{{ t("settings.update.notes") }}</h4>
        <p class="text-xs whitespace-pre-wrap select-text">{{ release.notes }}</p>
      </div>
    </div>

    <p v-if="progressLine" class="text-xs text-muted-foreground" data-update-progress>{{ progressLine }}</p>

    <!--
      A failed check is quoted rather than summarized, the same way the settings page quotes a failed
      write: the code is what turns "it did not work" into something somebody can search for.
    -->
    <p
      v-if="problem"
      data-update-problem
      class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs select-text"
    >
      <span>{{ problem.message }}</span>
      <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ problem.code }}</span>
    </p>

    <div class="flex items-start justify-between gap-4 border-t border-border pt-3">
      <div class="min-w-0 space-y-1">
        <p class="text-xs font-medium">{{ t("settings.update.autoLabel") }}</p>
        <p class="text-[11px] text-muted-foreground">{{ t("settings.update.autoHelp") }}</p>
      </div>
      <Switch
        :model-value="preferences.updateCheckOnStartup"
        :aria-label="t('settings.update.autoLabel')"
        @update:model-value="(value: boolean) => preferences.setUpdateCheckOnStartup(value)"
      />
    </div>
  </div>
</template>
