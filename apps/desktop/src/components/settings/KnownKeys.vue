<script setup lang="ts">
/**
 * The keys this device holds, and the way to stop holding one.
 *
 * Only the facts that identify a key are here — its name, its algorithm, its fingerprint, whether a
 * passphrase is held — because that is all the vault hands out about one. The list exists for the
 * same reason the trusted keys do: a key imported once should be reviewable later, and a key that
 * is no longer wanted should be removable without hunting for the session that imported it.
 *
 * Forgetting says what it costs: the sessions using that key cannot connect until they are given
 * another one. The count comes from the sessions themselves rather than from a guess.
 */
import { KeyRound, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { describeShellError } from "@/i18n/errors";
import { useKeysStore } from "@/stores/keys";
import { useSessionsStore } from "@/stores/sessions";
import type { KeyInfo } from "@/types/session";

const { t } = useI18n();
const keys = useKeysStore();
const library = useSessionsStore();

const removing = ref<KeyInfo | null>(null);
const passphrase = ref("");

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const problem = computed(() => (keys.failure ? describeShellError(translate, keys.failure) : null));

/** How many saved sessions would lose their way in if this key went. */
function sessionsUsing(keyRef: string | undefined): number {
  if (!keyRef) {
    return 0;
  }
  return library.sessions.filter((session) => session.auth.key?.keyRef === keyRef).length;
}

onMounted(async () => {
  await keys.load();
  // The count in the question is about sessions, so they have to be here before it is asked.
  if (library.sessions.length === 0) {
    await library.load();
  }
});
</script>

<template>
  <div class="space-y-2">
    <p v-if="problem" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
      <span>{{ problem.message }}</span>
      <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
    </p>

    <div class="flex items-center gap-2">
      <input
        v-model="passphrase"
        type="password"
        autocomplete="off"
        class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-xs focus:border-primary focus:outline-none"
        :placeholder="t('keys.passphrase')"
        :aria-label="t('keys.passphrase')"
      />
      <button
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1 text-xs transition-colors hover:bg-accent disabled:opacity-50"
        :disabled="keys.importing"
        @click="keys.importKey(t('keys.import'), passphrase === '' ? undefined : passphrase)"
      >
        <KeyRound :size="12" />
        {{ keys.importing ? t("keys.importing") : t("keys.import") }}
      </button>
    </div>

    <p v-if="keys.loaded && keys.entries.length === 0" class="text-xs text-muted-foreground">
      {{ t("keys.empty") }}
    </p>

    <div
      v-for="entry in keys.entries"
      :key="entry.keyRef ?? entry.fingerprint"
      class="flex items-start gap-3 rounded-md border border-border bg-muted/20 px-3 py-2.5"
    >
      <span class="mt-0.5 grid size-6 shrink-0 place-items-center rounded-md border border-border bg-background">
        <KeyRound :size="12" class="text-muted-foreground" />
      </span>

      <div class="min-w-0 flex-1">
        <p class="truncate text-xs">{{ entry.displayName }}</p>
        <p class="mt-1 font-mono text-[11px] break-all text-muted-foreground">{{ entry.fingerprint }}</p>
        <p class="mt-0.5 text-[10px] text-muted-foreground">
          {{ t("keys.algorithm", { algorithm: entry.algorithm, type: entry.hasPassphrase ? "passphrase" : "plain" }) }}
          · {{ t("keys.usedBy", { count: sessionsUsing(entry.keyRef) }) }}
        </p>
      </div>

      <button
        type="button"
        class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
        :title="t('keys.forget')"
        :aria-label="t('keys.forget')"
        @click="removing = entry"
      >
        <Trash2 :size="12" />
      </button>
    </div>

    <ConfirmDialog
      v-if="removing"
      :open="true"
      :title="t('keys.forgetTitle')"
      :message="
        t('keys.forgetMessage', {
          name: removing.displayName,
          count: sessionsUsing(removing.keyRef),
        })
      "
      :confirm-label="t('keys.forgetConfirm')"
      danger
      @confirm="
        keys.forget(removing.keyRef ?? '');
        removing = null;
      "
      @update:open="removing = null"
    />
  </div>
</template>
