<script setup lang="ts">
/**
 * The release channel, in the title bar: a place to look and a place to ask.
 *
 * The icon is there whether or not there is anything to say, because the question it answers — "is
 * this the newest build" — is one somebody asks *before* there is an answer, and pressing it is how
 * the answer arrives. What the tooltip carries is the state: an icon cannot tell "checking" apart from
 * "the channel could not be read", and those are two different things to do about it.
 *
 * A published release changes this entry rather than adding a second one: the version is then on
 * screen without a hover, and pressing it leads to the page that installs it. The percentage replaces
 * the version while an installation is running, which is the one moment this build has something more
 * interesting to say about an update than its number.
 *
 * Not drawn at all in a browser: there is no installed copy for an update to replace, which the page
 * about the build says in words instead.
 */
import { CloudDownload } from "@lucide/vue";
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { isDesktopRuntime } from "@/lib/runtime";
import { useTerminalsStore } from "@/stores/terminals";
import { useUpdatesStore } from "@/stores/updates";

const { t } = useI18n();
const updates = useUpdatesStore();
const terminals = useTerminalsStore();

/** How far an installation has got, or nothing while there is no download to measure. */
const percent = computed(() => {
  const progress = updates.progress;
  if (progress?.kind !== "downloading") {
    return null;
  }
  // The total is optional in the event: a server that sends no length gives nothing to divide by, and
  // a percentage invented from nothing would be one that moves backwards.
  const total = progress.total;
  if (total === undefined || total <= 0) {
    return null;
  }
  return Math.min(100, Math.round((progress.downloaded / total) * 100));
});

/** What the entry says it does: the tooltip and the accessible name, said once. */
const label = computed(() => {
  if (updates.asking) {
    return t("settings.update.checking");
  }
  if (updates.failure) {
    return t("shell.update.unreadable");
  }
  return t("settings.update.check");
});

async function ask(): Promise<void> {
  await updates.check();
}
</script>

<template>
  <button
    v-if="updates.release"
    type="button"
    data-update-entry
    data-update-available
    class="flex h-8 items-center gap-1.5 rounded-md bg-primary px-2 font-mono text-[11px] leading-none text-primary-foreground transition-colors hover:bg-primary/90"
    :title="t('shell.update.available', { version: updates.release.version })"
    :aria-label="t('shell.update.available', { version: updates.release.version })"
    @click="terminals.openSettings('about')"
  >
    <CloudDownload :size="13" />
    <span data-update-version>
      {{ updates.installing && percent !== null ? `${percent}%` : updates.release.version }}
    </span>
  </button>

  <button
    v-else-if="isDesktopRuntime()"
    type="button"
    data-update-entry
    class="grid size-8 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:cursor-default disabled:opacity-60 disabled:hover:bg-transparent disabled:hover:text-muted-foreground"
    :title="label"
    :aria-label="label"
    :disabled="updates.asking"
    @click="ask()"
  >
    <CloudDownload :size="15" />
  </button>
</template>
