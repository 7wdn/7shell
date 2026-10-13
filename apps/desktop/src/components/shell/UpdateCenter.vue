<script setup lang="ts">
/**
 * The update centre: what the channel found, and the four things somebody can do about it.
 *
 * It opens from the entry in the title bar — the answer to "is this the newest build" is a page, not
 * a hover — and it asks the channel again as it opens, so what is on screen is now's answer rather
 * than the answer from however long ago the window started.
 *
 * What it deliberately does in two steps is the download and the install. An archive arriving is not
 * the same event as the window being replaced: the first is invisible work that can happen while
 * somebody is in the middle of something, the second is the thing that takes the window away. So the
 * footer downloads first, and "restart and update" is the button that a person presses when they are
 * ready — the same split the shell keeps in its own state.
 *
 * It is also where "not this version" is decided, which is a version and not a switch: turning the
 * checks off would stop news of everything after it too.
 */
import { Download, LoaderCircle, RefreshCw, RotateCw, TriangleAlert, X } from "@lucide/vue";
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import { computed, watch } from "vue";
import { useI18n } from "vue-i18n";

import UpdateNotes from "@/components/shell/UpdateNotes.vue";
import { describeShellError } from "@/i18n/errors";
import { useUpdatesStore } from "@/stores/updates";

const { t, te } = useI18n();
const updates = useUpdatesStore();

const open = computed({
  get: () => updates.dialogOpen,
  set: (value: boolean) => {
    updates.dialogOpen = value;
  },
});

const release = computed(() => updates.release);

/**
 * Asks the channel as the window opens.
 *
 * The answer on screen should be now's answer: the window may be opened an hour after the startup
 * check, and "nothing newer" from an hour ago is a claim this build cannot stand behind.
 */
watch(open, (showing) => {
  if (showing) {
    void updates.check();
  }
});
const answer = computed(() => updates.answer);

/** What the line above the notes says: the release, the newest, or nothing yet. */
const headline = computed(() => {
  if (release.value) {
    return t("settings.update.publishedOver", {
      version: release.value.version,
      current: answer.value?.currentVersion ?? "",
    });
  }
  if (answer.value?.kind === "upToDate") {
    return t("settings.update.newest", { version: answer.value.currentVersion });
  }
  if (answer.value?.kind === "notInstalled") {
    return t("settings.update.notInstalled");
  }
  return null;
});

/** Whether this version has been downloaded and is waiting for a restart. */
const ready = computed(() => release.value !== null && updates.downloadedVersion === release.value.version);

const problem = computed(() => (updates.failure ? describeShellError(translate, updates.failure) : null));

/** How far a download has got, in percent, while one is running. */
const percent = computed(() => {
  const progress = updates.progress;
  if (progress?.kind !== "downloading") {
    return null;
  }
  const total = progress.total;
  if (total === undefined || total <= 0) {
    return null;
  }
  return Math.min(100, Math.round((progress.downloaded / total) * 100));
});

function translate(key: string, params?: Record<string, unknown>): string {
  return te(key) ? t(key, params ?? {}) : key;
}

/** Downloads and closes the window, so the work can finish behind it: the entry carries the count. */
async function downloadInBackground(): Promise<void> {
  open.value = false;
  await updates.download();
}
</script>

<template>
  <DialogRoot :open="open" @update:open="(value: boolean) => (open = value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        data-update-centre
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[min(38rem,calc(100vh-3rem))] w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <header class="flex items-center gap-2 border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">{{ t("settings.update.centre") }}</DialogTitle>
          <span
            v-if="release"
            data-update-badge
            class="rounded-full bg-primary/15 px-2 py-0.5 text-[11px] leading-4 text-primary"
          >
            {{ t("settings.update.availableBadge") }}
          </span>
          <DialogDescription class="sr-only">{{ t("settings.update.centreHelp") }}</DialogDescription>
          <DialogClose
            class="ml-auto grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('common.action.close')"
          >
            <X :size="14" />
          </DialogClose>
        </header>

        <div class="min-h-0 flex-1 overflow-y-auto px-4 py-3">
          <p v-if="headline" data-update-headline class="text-sm leading-relaxed">{{ headline }}</p>

          <div
            v-if="release?.notes"
            data-update-notes
            class="mt-3 max-h-64 overflow-y-auto rounded-md border border-border bg-muted/30 p-3"
          >
            <UpdateNotes :markdown="release.notes" />
          </div>

          <p
            v-if="updates.asking"
            data-update-asking
            class="mt-3 flex items-center gap-2 text-xs text-muted-foreground"
          >
            <LoaderCircle :size="13" class="animate-spin" />
            {{ t("settings.update.checking") }}
          </p>

          <p v-if="ready && !updates.installing" data-update-ready class="mt-3 text-sm">
            {{ t("settings.update.ready", { version: release?.version ?? "" }) }}
          </p>

          <div v-if="updates.installing" data-update-progress class="mt-3 space-y-1">
            <p class="text-xs text-muted-foreground">
              {{
                updates.progress?.kind === "installing"
                  ? t("settings.update.installing")
                  : t("settings.update.downloadingUnknown", { done: "" })
              }}
            </p>
            <div v-if="percent !== null" class="h-1 overflow-hidden rounded-full bg-muted">
              <div class="h-full bg-primary transition-[width]" :style="{ width: `${percent}%` }"></div>
            </div>
          </div>

          <p
            v-if="problem"
            data-update-problem
            role="alert"
            class="mt-3 flex items-start gap-2 text-xs text-destructive"
          >
            <TriangleAlert :size="14" class="mt-0.5 shrink-0" />
            <span>{{ problem }}</span>
          </p>
        </div>

        <footer class="flex flex-wrap items-center gap-2 border-t border-border px-4 py-3">
          <button
            v-if="release"
            type="button"
            data-update-ignore
            class="mr-auto rounded-md px-2 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            @click="updates.ignore(release.version).then(() => (open = false))"
          >
            {{ t("settings.update.ignoreVersion") }}
          </button>

          <button
            type="button"
            data-update-page
            class="rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent"
            @click="updates.openReleasePage(release?.version ?? answer?.currentVersion ?? '')"
          >
            {{ t("settings.update.openReleasePage") }}
          </button>

          <button
            v-if="release && !ready"
            type="button"
            data-update-background
            class="ml-auto rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent"
            @click="downloadInBackground()"
          >
            {{ t("settings.update.downloadInBackground") }}
          </button>

          <button
            v-if="release && !ready"
            type="button"
            data-update-download
            :disabled="updates.installing"
            class="inline-flex items-center gap-1.5 rounded-md bg-primary px-2.5 py-1 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:opacity-60"
            @click="updates.download()"
          >
            <LoaderCircle v-if="updates.installing" :size="13" class="animate-spin" />
            <Download v-else :size="13" />
            {{ percent === null ? t("settings.update.download") : `${percent}%` }}
          </button>

          <button
            v-if="release && ready"
            type="button"
            data-update-restart
            class="ml-auto inline-flex items-center gap-1.5 rounded-md bg-primary px-2.5 py-1 text-xs text-primary-foreground transition-colors hover:bg-primary/90"
            @click="updates.install()"
          >
            <RotateCw :size="13" />
            {{ t("settings.update.restartAndUpdate") }}
          </button>

          <button
            v-if="!release"
            type="button"
            data-update-recheck
            class="ml-auto inline-flex items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent"
            @click="updates.check()"
          >
            <RefreshCw :size="13" :class="updates.asking ? 'animate-spin' : undefined" />
            {{ t("settings.update.check") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
