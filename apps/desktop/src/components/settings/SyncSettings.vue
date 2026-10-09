<script setup lang="ts">
/**
 * Sync, and the two halves of it this build has: a WebDAV server, and a file.
 *
 * Three tabs, one of which says it is not here yet rather than opening onto nothing.
 *
 * The two panels are laid out the same way on purpose, because they are the same three parts in the
 * same order — what this is, the secret that is kept for it, and the two things somebody does with
 * it. Where they differ is where the config lives: a server's address, user name and path are
 * settings and are written by the page's own Apply, while the file panel needs nothing but a
 * passphrase.
 *
 * **Every server action commits first.** The shell reads the address out of the settings file, so a
 * test pressed with an unsaved address on screen would dial the address that was saved — which is
 * exactly the mistake somebody makes once and then distrusts the button. That is also what the
 * application this page is modelled on does, in its own way: it writes its fields down before it
 * uses them.
 *
 * A password and a passphrase are kept apart and are neither of them in this store. The password
 * authenticates to the server; the passphrase seals what is uploaded and never leaves this machine.
 * Both live in the vault, and this page only asks whether one is there.
 */
import { Archive, Cloud, Download, Eye, EyeOff, Loader2, Upload, X } from "@lucide/vue";
import { computed, onMounted, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import SyncIncludeDialog from "@/components/settings/SyncIncludeDialog.vue";
import SyncRestoreDialog from "@/components/settings/SyncRestoreDialog.vue";
import { describeShellError } from "@/i18n/errors";
import { backupReceipt, restoreReceipt } from "@/lib/sync/receipt";
import { useCommandsStore } from "@/stores/commands";
import { usePreferencesStore } from "@/stores/preferences";
import { useSessionsStore } from "@/stores/sessions";
import { useSyncStore, type ReadBackup } from "@/stores/sync";
import { useWordsStore } from "@/stores/words";
import type { SyncSelection } from "@/types/sync";

const { t } = useI18n();

const preferences = usePreferencesStore();
const library = useSessionsStore();
const commands = useCommandsStore();
const words = useWordsStore();
const sync = useSyncStore();

/** Which of the three the page is showing. */
const tab = ref<"webdav" | "snippets" | "local">("webdav");
/** The file panel's passphrase, which seals a backup and opens a sealed one. */
const passphrase = ref("");
/** The server's passphrase, which is a different thing: it never reaches the server. */
const secretsPassphrase = ref("");
/** The server's password, in memory only — the vault keeps it if somebody asks. */
const password = ref("");
/** Whether the password should be kept, which is drawn from whether one already is. */
const rememberPassword = ref(false);
/** Whether either secret is being shown, for a field somebody has to check their typing in. */
const passwordShown = ref(false);
const secretsShown = ref(false);

const includeOpen = ref(false);
const restoreOpen = ref(false);
/** What a read brought back — from the server, or from a file — for the dialog that shows it. */
const downloaded = ref<ReadBackup | null>(null);
/** What an action did, said under the buttons. */
const notice = ref("");

/** The name a backup is offered as, before the file dialog asks where it goes. */
const BACKUP_FILE_NAME = "7shell-backup.json";

const working = computed(() => sync.busy !== "");
/** A passphrase long enough to seal something under, which is the shell's own floor. */
const canSeal = computed(() => secretsPassphrase.value.length >= 4 || sync.service?.hasPassphrase === true);
/** Whether there is an address to talk to. */
const hasEndpoint = computed(() => preferences.syncEndpoint.trim() !== "");

const problem = computed(() => (sync.failure ? describeShellError(translate, sync.failure).message : ""));

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** The text a field now holds, for the three fields whose writes go through the store. */
function typed(event: Event): string {
  return (event.target as HTMLInputElement).value;
}

onMounted(() => {
  void sync.load();
  void sync.loadService();
});

// The box beside the password follows what the vault holds rather than a preference of its own:
// "kept" and "ticked" are the same state, and two places to look for it is how they come apart.
watch(
  () => sync.service?.hasPassword,
  (kept) => {
    if (kept !== undefined) {
      rememberPassword.value = kept;
    }
  },
  { immediate: true },
);

/**
 * Writes the settings the shell is about to read, and the two secrets the vault is about to hold.
 *
 * Called before every server action, which is what makes a test test what is on screen. The password
 * moves only when the box says it should, and only from what was typed: a field left empty is not a
 * password somebody cleared, it is one they did not need to type again.
 */
async function prepareServer(): Promise<boolean> {
  if (!hasEndpoint.value) {
    notice.value = t("sync.noServer");
    return false;
  }
  // The commit is the page's own Apply: the address has to be in the file before the shell dials it.
  await preferences.commit();

  if (rememberPassword.value && password.value !== "") {
    await sync.keepPassword(password.value);
    password.value = "";
  } else if (!rememberPassword.value && sync.service?.hasPassword) {
    await sync.keepPassword(null);
  }

  if (preferences.syncIncludeSecrets && secretsPassphrase.value !== "") {
    await sync.keepPassphrase(secretsPassphrase.value);
    secretsPassphrase.value = "";
  } else if (!preferences.syncIncludeSecrets && sync.service?.hasPassphrase) {
    await sync.keepPassphrase(null);
  }

  return true;
}

async function testServer(): Promise<void> {
  notice.value = "";
  if (!(await prepareServer())) {
    return;
  }
  if (await sync.testServer()) {
    notice.value = t("sync.testOk");
  }
}

function uploadToServer(selection: SyncSelection): void {
  includeOpen.value = false;
  void (async () => {
    if (!(await prepareServer())) {
      return;
    }
    const receipt = await sync.uploadToServer(
      selection,
      secretsPassphrase.value === "" ? null : secretsPassphrase.value,
    );
    if (receipt) {
      notice.value = backupReceipt(receipt.contents, receipt.sealed, receipt.remotePath, t);
    }
  })();
}

function downloadFromServer(): void {
  void (async () => {
    if (!(await prepareServer())) {
      return;
    }
    const outcome = await sync.downloadFromServer(secretsPassphrase.value === "" ? null : secretsPassphrase.value);
    if (!outcome) {
      return;
    }
    if (outcome.kind === "nothingYet") {
      notice.value = t("sync.nothingOnServer");
      return;
    }
    if (outcome.kind === "needsPassphrase") {
      // The field that answers this is on the panel behind the message, which is why there is no
      // dialog for it: the sentence and the box are one thing.
      notice.value = t("sync.secretsPassphraseKept");
      return;
    }
    downloaded.value = { fileName: preferences.syncRemotePath.trim() || t("sync.tabWebdav"), catalog: outcome.catalog };
    restoreOpen.value = true;
  })();
}

function applyFromServer(replace: boolean): void {
  restoreOpen.value = false;
  void (async () => {
    const summary = await sync.applyFromServer(
      replace,
      secretsPassphrase.value === "" ? null : secretsPassphrase.value,
    );
    if (!summary) {
      return;
    }
    notice.value = restoreReceipt(summary, t);
    await reloadAfterRestore(summary.contents.settings > 0);
  })();
}

/** Everything an import can have written, read again. */
async function reloadAfterRestore(settingsChanged: boolean): Promise<void> {
  await Promise.all([library.load(), commands.load(), words.load()]);
  if (settingsChanged) {
    await preferences.load();
  }
}

// -- The file panel, which is the same three parts with nothing to configure ----------

function writeBackup(selection: SyncSelection): void {
  includeOpen.value = false;
  void (async () => {
    const receipt = await sync.writeBackup(
      t("sync.export"),
      BACKUP_FILE_NAME,
      selection,
      passphrase.value.length >= 4 ? passphrase.value : null,
    );
    if (receipt) {
      notice.value = backupReceipt(receipt.contents, receipt.sealed, receipt.fileName, t);
    }
  })();
}

function readBackup(): void {
  void (async () => {
    const read = await sync.readBackup(t("sync.import"), passphrase.value === "" ? null : passphrase.value);
    if (read) {
      downloaded.value = read;
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
    await reloadAfterRestore(summary.contents.settings > 0);
  })();
}
</script>

<template>
  <section class="space-y-4">
    <!--
      The three things a sync page is. One of them is not here yet, and a tab that opens onto nothing
      would be a worse lie than a tab that says so.
    -->
    <div class="flex gap-1 rounded-md border border-border bg-muted/30 p-1" role="tablist">
      <button
        type="button"
        role="tab"
        :aria-selected="tab === 'webdav'"
        class="flex-1 rounded px-3 py-1.5 text-xs"
        :class="tab === 'webdav' ? 'bg-card text-foreground shadow-sm' : 'text-muted-foreground'"
        @click="tab = 'webdav'"
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

    <p v-if="tab === 'snippets'" class="text-[11px] leading-relaxed text-muted-foreground">
      {{ t("sync.notYet") }}
    </p>

    <template v-if="tab === 'webdav'">
      <div class="space-y-3 rounded-md border border-border px-3 py-3">
        <div class="space-y-1">
          <h4 class="flex items-center gap-1.5 text-xs font-medium">
            <Cloud :size="13" class="text-muted-foreground" />
            {{ t("sync.webdavTitle") }}
          </h4>
          <p class="text-xs leading-relaxed text-muted-foreground">{{ t("sync.webdavDescription") }}</p>
        </div>

        <div class="space-y-2">
          <div>
            <label class="block text-xs" for="sync-endpoint">{{ t("sync.endpoint") }}</label>
            <input
              id="sync-endpoint"
              :value="preferences.syncEndpoint"
              type="text"
              spellcheck="false"
              autocomplete="off"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none select-text"
              :placeholder="t('sync.endpointPlaceholder')"
              :disabled="working"
              @input="preferences.setSyncEndpoint(typed($event))"
            />
          </div>

          <div class="grid gap-2 sm:grid-cols-2">
            <div>
              <label class="block text-xs" for="sync-username">{{ t("sync.username") }}</label>
              <input
                id="sync-username"
                :value="preferences.syncUsername"
                type="text"
                spellcheck="false"
                autocomplete="off"
                class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none select-text"
                :disabled="working"
                @input="preferences.setSyncUsername(typed($event))"
              />
            </div>
            <div>
              <label class="block text-xs" for="sync-password">{{ t("sync.password") }}</label>
              <div class="relative mt-1">
                <input
                  id="sync-password"
                  v-model="password"
                  :type="passwordShown ? 'text' : 'password'"
                  autocomplete="off"
                  class="w-full rounded-md border border-border bg-background px-3 py-2 pr-9 text-sm focus:outline-none"
                  :placeholder="t('sync.passwordPlaceholder')"
                  :disabled="working"
                />
                <button
                  type="button"
                  class="absolute inset-y-0 right-0 grid w-8 place-items-center text-muted-foreground hover:text-foreground"
                  :aria-label="t('sync.password')"
                  @click="passwordShown = !passwordShown"
                >
                  <EyeOff v-if="passwordShown" :size="13" />
                  <Eye v-else :size="13" />
                </button>
              </div>
            </div>
          </div>

          <label class="flex items-center gap-2 text-xs">
            <input
              v-model="rememberPassword"
              type="checkbox"
              class="size-3.5 shrink-0 accent-primary"
              :disabled="working"
            />
            {{ t("sync.rememberPassword") }}
          </label>
          <button
            v-if="sync.service?.hasPassword"
            type="button"
            class="flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
            :disabled="working"
            @click="
              async () => {
                await sync.keepPassword(null);
                rememberPassword = false;
                password = '';
              }
            "
          >
            <X :size="11" />
            {{ t("sync.forgetPassword") }}
          </button>

          <div>
            <label class="block text-xs" for="sync-remote-path">{{ t("sync.remotePath") }}</label>
            <input
              id="sync-remote-path"
              :value="preferences.syncRemotePath"
              type="text"
              spellcheck="false"
              autocomplete="off"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none select-text"
              :disabled="working"
              @input="preferences.setSyncRemotePath(typed($event))"
            />
            <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">{{ t("sync.remotePathHint") }}</p>
          </div>
        </div>

        <p v-if="problem" class="text-[11px] text-destructive" role="alert">{{ problem }}</p>
        <p v-else-if="notice" class="text-[11px] text-muted-foreground">{{ notice }}</p>

        <!-- Testing first, then the two directions: a person who is not sure their address is right
             presses the one that changes nothing, and it is the leftmost of the three. -->
        <div class="flex flex-wrap items-center justify-end gap-2">
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="working"
            @click="testServer()"
          >
            <Loader2 v-if="sync.busy === 'test'" :size="12" class="animate-spin" />
            {{ t("sync.test") }}
          </button>
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="working"
            @click="downloadFromServer()"
          >
            <Loader2 v-if="sync.busy === 'download'" :size="12" class="animate-spin" />
            <Download v-else :size="12" />
            {{ t("sync.download") }}
          </button>
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="working"
            @click="includeOpen = true"
          >
            <Loader2 v-if="sync.busy === 'upload'" :size="12" class="animate-spin" />
            <Upload v-else :size="12" />
            {{ t("sync.upload") }}
          </button>
        </div>
      </div>

      <!-- The one secret that never reaches the server, kept apart from the one that does. -->
      <div class="space-y-2 rounded-md border border-border px-3 py-3">
        <label class="flex items-start gap-2">
          <input
            :model-value="preferences.syncIncludeSecrets"
            type="checkbox"
            class="mt-0.5 size-3.5 shrink-0 accent-primary"
            :disabled="working"
            @change="preferences.setSyncIncludeSecrets(($event.target as HTMLInputElement).checked)"
          />
          <span class="min-w-0">
            <span class="block text-xs">{{ t("sync.secretsToggle") }}</span>
            <span class="mt-0.5 block text-[11px] leading-relaxed text-muted-foreground">
              {{ t("sync.secretsHint") }}
            </span>
          </span>
        </label>

        <div v-if="preferences.syncIncludeSecrets">
          <label class="block text-xs" for="sync-secrets-passphrase">{{ t("sync.secretsPassphrase") }}</label>
          <div class="relative mt-1">
            <input
              id="sync-secrets-passphrase"
              v-model="secretsPassphrase"
              :type="secretsShown ? 'text' : 'password'"
              autocomplete="off"
              class="w-full rounded-md border border-border bg-background px-3 py-2 pr-9 text-sm focus:outline-none"
              :placeholder="t('sync.secretsPassphrasePlaceholder')"
              :disabled="working"
            />
            <button
              type="button"
              class="absolute inset-y-0 right-0 grid w-8 place-items-center text-muted-foreground hover:text-foreground"
              :aria-label="t('sync.secretsPassphrase')"
              @click="secretsShown = !secretsShown"
            >
              <EyeOff v-if="secretsShown" :size="13" />
              <Eye v-else :size="13" />
            </button>
          </div>
          <p v-if="sync.service?.hasPassphrase" class="mt-1 text-[11px] leading-relaxed text-muted-foreground">
            {{ t("sync.secretsPassphraseKept") }}
          </p>
          <button
            v-if="sync.service?.hasPassphrase"
            type="button"
            class="mt-1 flex items-center gap-1 text-[11px] text-muted-foreground hover:text-foreground"
            :disabled="working"
            @click="
              async () => {
                await sync.keepPassphrase(null);
                secretsPassphrase = '';
              }
            "
          >
            <X :size="11" />
            {{ t("sync.forgetPassphrase") }}
          </button>
        </div>
      </div>
    </template>

    <template v-else-if="tab === 'local'">
      <div class="space-y-3 rounded-md border border-border px-3 py-3">
        <div class="space-y-1">
          <h4 class="flex items-center gap-1.5 text-xs font-medium">
            <Archive :size="13" class="text-muted-foreground" />
            {{ t("sync.backupTitle") }}
          </h4>
          <p class="text-xs leading-relaxed text-muted-foreground">{{ t("sync.backupDescription") }}</p>
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
          <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">{{ t("sync.passphraseHint") }}</p>
        </div>

        <p v-if="problem" class="text-[11px] text-destructive" role="alert">{{ problem }}</p>
        <p v-else-if="notice" class="text-[11px] text-muted-foreground">{{ notice }}</p>

        <div class="flex justify-end gap-2">
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="working"
            @click="readBackup()"
          >
            <Loader2 v-if="sync.busy === 'import'" :size="12" class="animate-spin" />
            <Download v-else :size="12" />
            {{ t("sync.import") }}
          </button>
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
        </div>
      </div>
    </template>

    <SyncIncludeDialog
      :open="includeOpen"
      :busy="sync.busy === 'export' || sync.busy === 'upload'"
      :available="sync.available"
      :can-seal="tab === 'webdav' ? canSeal : passphrase.length >= 4"
      @confirm="tab === 'webdav' ? uploadToServer($event) : writeBackup($event)"
      @update:open="includeOpen = $event"
    />

    <SyncRestoreDialog
      :open="restoreOpen"
      :busy="sync.busy === 'import' || sync.busy === 'download'"
      :read="downloaded"
      @confirm="tab === 'webdav' ? applyFromServer($event) : restore($event)"
      @update:open="
        (open: boolean) => {
          restoreOpen = open;
          if (!open) {
            downloaded = null;
            sync.forgetRead();
          }
        }
      "
    />
  </section>
</template>
