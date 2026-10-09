<script setup lang="ts">
/**
 * Sync, and the one half of it this build has: backups to a file.
 *
 * Three tabs, because the feature is three things and hiding two of them would make the page look
 * finished when it is not — the two that have not landed yet say so rather than opening onto
 * nothing.
 *
 * The passphrase is on the panel rather than in the dialog behind it, because it answers two
 * questions: it is what seals a backup being written, and it is what opens a sealed one being read.
 * A wrong one lands beside the field that was typed in, and the shell is still holding the file, so
 * the answer is typed again rather than the file chosen again.
 *
 * After a restore this page reads the library again — the sessions, the commands, the words and the
 * settings, whichever of them changed. Only the shell knows which ids a restore had to replace, so
 * anything less than reading again would be a second guess at what is on disk.
 */
import { Loader2, Upload } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import SyncIncludeDialog from "@/components/settings/SyncIncludeDialog.vue";
import SyncRestoreDialog from "@/components/settings/SyncRestoreDialog.vue";
import { describeShellError } from "@/i18n/errors";
import { backupReceipt, restoreReceipt } from "@/lib/sync/receipt";
import { useCommandsStore } from "@/stores/commands";
import { usePreferencesStore } from "@/stores/preferences";
import { useSessionsStore } from "@/stores/sessions";
import { useSyncStore } from "@/stores/sync";
import { useWordsStore } from "@/stores/words";
import type { SyncSelection } from "@/types/sync";

const { t } = useI18n();

const preferences = usePreferencesStore();
const library = useSessionsStore();
const commands = useCommandsStore();
const words = useWordsStore();
const sync = useSyncStore();

/** Which of the three the page is showing. */
const tab = ref<"webdav" | "snippets" | "local">("local");
/** The passphrase field, which seals a backup and opens a sealed one. */
const passphrase = ref("");
const includeOpen = ref(false);
const restoreOpen = ref(false);
/** What an import did, said here because this page is where it happened. */
const notice = ref("");

/** The name a backup is offered as, before the file dialog asks where it goes. */
const BACKUP_FILE_NAME = "7shell-backup.json";

const working = computed(() => sync.busy !== "");
const canSeal = computed(() => passphrase.value.length >= 4);

const problem = computed(() => (sync.failure ? describeShellError(translate, sync.failure).message : ""));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

onMounted(() => {
  void sync.load();
});

function writeBackup(selection: SyncSelection): void {
  includeOpen.value = false;
  void (async () => {
    const receipt = await sync.writeBackup(
      t("sync.export"),
      BACKUP_FILE_NAME,
      selection,
      canSeal.value ? passphrase.value : null,
    );
    if (receipt) {
      notice.value = backupReceipt(receipt.contents, receipt.sealed, receipt.fileName, t);
    }
  })();
}

/**
 * Reads a backup, and either says what is inside it or asks for what is missing.
 *
 * An answer of "sealed" is not shown: the field that answers it is on this panel, so the press of
 * this button *is* the question. What comes back instead is either the dialog or, when the
 * passphrase was wrong, the line beside the field.
 */
function readBackup(): void {
  void (async () => {
    const read = await sync.readBackup(t("sync.import"), passphrase.value === "" ? null : passphrase.value);
    if (read) {
      restoreOpen.value = true;
    }
  })();
}

function restore(replace: boolean): void {
  restoreOpen.value = false;
  void (async () => {
    const summary = await sync.applyBackup(replace, passphrase.value === "" ? null : passphrase.value);
    if (!summary) {
      return;
    }
    notice.value = restoreReceipt(summary, t);

    // Everything a restore can have written, read again — the settings included, because a backup
    // may carry them and this page would otherwise show the theme it had a moment ago.
    await Promise.all([library.load(), commands.load(), words.load()]);
    if (summary.contents.settings > 0) {
      await preferences.load();
    }
  })();
}
</script>

<template>
  <section class="space-y-4">
    <div class="space-y-1">
      <h3 class="text-sm font-medium">{{ t("sync.title") }}</h3>
      <p class="text-xs leading-relaxed text-muted-foreground">{{ t("sync.description") }}</p>
    </div>

    <!--
      The three things a sync page is. Two of them are not here yet, and a tab that opens onto
      nothing would be a worse lie than a tab that says so.
    -->
    <div class="flex gap-1 rounded-md border border-border bg-muted/30 p-1" role="tablist">
      <button
        type="button"
        role="tab"
        class="flex-1 rounded px-3 py-1.5 text-xs text-muted-foreground disabled:opacity-60"
        disabled
        :title="t('sync.later')"
      >
        {{ t("sync.tabWebdav") }}
      </button>
      <button
        type="button"
        role="tab"
        class="flex-1 rounded px-3 py-1.5 text-xs text-muted-foreground disabled:opacity-60"
        disabled
        :title="t('sync.later')"
      >
        {{ t("sync.tabSnippets") }}
      </button>
      <button
        type="button"
        role="tab"
        :aria-selected="tab === 'local'"
        class="flex-1 rounded px-3 py-1.5 text-xs"
        :class="tab === 'local' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground'"
        @click="tab = 'local'"
      >
        {{ t("sync.tabLocal") }}
      </button>
    </div>

    <div class="space-y-3 rounded-md border border-border bg-muted/20 px-3 py-3">
      <div class="space-y-1">
        <p class="text-xs">{{ t("sync.backupDescription") }}</p>
        <p class="text-[11px] leading-relaxed text-muted-foreground">
          {{ t("sync.passphraseHint") }}
        </p>
      </div>

      <div>
        <label class="block text-xs" for="sync-passphrase">{{ t("sync.passphrase") }}</label>
        <input
          id="sync-passphrase"
          v-model="passphrase"
          type="password"
          autocomplete="off"
          class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
          :placeholder="t('sync.passphrasePlaceholder')"
          :disabled="working"
        />
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <button
          type="button"
          class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
          :disabled="working"
          @click="includeOpen = true"
        >
          <Loader2 v-if="sync.busy === 'export'" :size="12" class="animate-spin" />
          <Upload v-else :size="12" />
          {{ t("sync.export") }}
        </button>
        <button
          type="button"
          class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
          :disabled="working"
          @click="readBackup()"
        >
          <Loader2 v-if="sync.busy === 'import'" :size="12" class="animate-spin" />
          {{ t("sync.import") }}
        </button>
      </div>

      <!-- The line under the buttons: a refusal beside the field that caused it, or what just
           happened. One place, because a sentence here and a toast for the same thing is two. -->
      <p v-if="problem" class="text-[11px] text-destructive" role="alert">{{ problem }}</p>
      <p v-else-if="notice" class="text-[11px] text-muted-foreground">{{ notice }}</p>
    </div>

    <SyncIncludeDialog
      :open="includeOpen"
      :busy="sync.busy === 'export'"
      :available="sync.available"
      :can-seal="canSeal"
      @confirm="writeBackup($event)"
      @update:open="includeOpen = $event"
    />

    <SyncRestoreDialog
      :open="restoreOpen"
      :busy="sync.busy === 'import'"
      :read="sync.read"
      @confirm="restore($event)"
      @update:open="
        (open: boolean) => {
          restoreOpen = open;
          if (!open) sync.forgetRead();
        }
      "
    />
  </section>
</template>
