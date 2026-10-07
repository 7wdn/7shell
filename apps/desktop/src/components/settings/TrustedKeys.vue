<script setup lang="ts">
/**
 * The keys this device trusts, and the way to stop trusting one.
 *
 * It is a list rather than a setting, which is why it lives in a category of its own: nobody
 * chooses a trusted key, they end up with one by answering a prompt — and the thing worth being
 * able to do afterwards is read the fingerprints that were accepted and take one back.
 *
 * Forgetting asks first. It does not destroy anything, but its consequence is invisible until the
 * next connection, when the prompt comes back — and "why is it asking me again" is a worse surprise
 * than one question now.
 */
import { ShieldCheck, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { describeShellError } from "@/i18n/errors";
import { useKnownHostsStore } from "@/stores/knownHosts";
import type { KnownHostEntry } from "@/types/session";

const { t } = useI18n();
const keys = useKnownHostsStore();

const removing = ref<KnownHostEntry | null>(null);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const problem = computed(() => (keys.failure ? describeShellError(translate, keys.failure) : null));

/** The entries, most recently trusted first: the one somebody is looking for is usually the last. */
const entries = computed(() => [...keys.entries].sort((left, right) => right.addedAt - left.addedAt));

function trustedAt(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString();
}

onMounted(() => void keys.load());
</script>

<template>
  <!--
    Readings, inside a page that is otherwise chrome: a fingerprint is checked against another
    machine's by eye, and a host and port are pasted into a terminal — so this list keeps its text
    where the settings page around it refuses selection. The forgetting buttons are still buttons.
  -->
  <div class="space-y-2 select-text">
    <p v-if="problem" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
      <span>{{ problem.message }}</span>
      <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
    </p>

    <p v-if="keys.loaded && entries.length === 0" class="text-xs text-muted-foreground">
      {{ t("knownHosts.empty") }}
    </p>

    <div
      v-for="entry in entries"
      :key="`${entry.host}:${entry.port}`"
      class="flex items-start gap-3 rounded-md border border-border bg-muted/20 px-3 py-2.5"
    >
      <span class="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md border border-border bg-background">
        <ShieldCheck :size="12" class="text-muted-foreground" />
      </span>

      <div class="min-w-0 flex-1">
        <p class="font-mono text-xs">{{ entry.host }}:{{ entry.port }}</p>
        <p class="mt-1 font-mono text-[11px] break-all text-muted-foreground">{{ entry.fingerprint }}</p>
        <p class="mt-0.5 text-[10px] text-muted-foreground">
          {{ t("knownHosts.trustedAt", { when: trustedAt(entry.addedAt) }) }} · {{ entry.keyType }}
        </p>
      </div>

      <button
        type="button"
        class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
        :title="t('knownHosts.forget')"
        :aria-label="t('knownHosts.forget')"
        @click="removing = entry"
      >
        <Trash2 :size="12" />
      </button>
    </div>

    <ConfirmDialog
      v-if="removing"
      :open="true"
      :title="t('knownHosts.forgetTitle')"
      :message="t('knownHosts.forgetMessage', { host: `${removing.host}:${removing.port}` })"
      :confirm-label="t('knownHosts.forgetConfirm')"
      danger
      @confirm="
        keys.forget(removing.host, removing.port);
        removing = null;
      "
      @update:open="removing = null"
    />
  </div>
</template>
