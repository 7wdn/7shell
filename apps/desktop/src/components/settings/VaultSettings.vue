<script setup lang="ts">
/**
 * The vault, from the settings page: its master password, the lock, and the libraries kept beside it.
 *
 * The three things somebody comes looking for on the security page, and all three are about the file
 * rather than about a session:
 *
 * * **Changing the master password** re-encrypts the vault, and the current password is asked for
 *   again even though the vault is open — an unlocked window is not by itself permission to change
 *   what opens it. Both new fields are checked here before anything is derived, because the backend's
 *   answer to a short password is a derived key, not a refusal.
 * * **Locking** drops the derived key and the document. Open terminals keep running: they are
 *   authenticated already, and closing somebody's work because they locked the password store would
 *   be the lock doing damage. What follows is the gate, which is where this page is drawn from.
 * * **Archived vaults** are the libraries a forgotten password left behind, and the same dialog the
 *   unlock screen opens — here, though, the vault is open, so importing what one holds works and
 *   switching says why it cannot.
 *
 * Nothing here shows a password back: the fields are cleared when the change succeeds, and a refusal
 * is drawn under the form that caused it with the code the backend sent.
 */
import { History, LoaderCircle, Lock } from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import VaultHistoryDialog from "@/components/vault/VaultHistoryDialog.vue";
import { describeShellError } from "@/i18n/errors";
import { MINIMUM_PASSWORD_LENGTH } from "@/lib/vault/password";
import { useVaultStore } from "@/stores/vault";

const { t } = useI18n();
const vault = useVaultStore();

const current = ref("");
const next = ref("");
const confirmation = ref("");
const busy = ref(false);
/** Whether the last change went through, which is worth saying: the file is different now. */
const changed = ref(false);
const failure = ref<ReturnType<typeof describeShellError> | null>(null);

const historyOpen = ref(false);

/** Shown while typing rather than on submit, so the rule is visible at the moment it is broken. */
const tooShort = computed(() => next.value.length > 0 && next.value.length < MINIMUM_PASSWORD_LENGTH);
const mismatched = computed(() => confirmation.value.length > 0 && confirmation.value !== next.value);
const unchanged = computed(() => next.value.length > 0 && next.value === current.value);
const canSubmit = computed(
  () =>
    current.value.length > 0 &&
    next.value.length >= MINIMUM_PASSWORD_LENGTH &&
    next.value === confirmation.value &&
    !unchanged.value,
);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** Clears the fields on anything that ends the attempt, so a mistyped password is not left on screen. */
function forget(): void {
  current.value = "";
  next.value = "";
  confirmation.value = "";
}

/**
 * The refusal goes away with the password it was about.
 *
 * Most of what can fail here — the wrong current password above all — is a sentence about what is in
 * that field, and leaving it under a field somebody is already retyping is the page arguing with
 * itself. Editing either of the new fields does not clear it: what could not be written is a fact
 * about the file rather than about them, and it is answered by pressing the button again.
 */
watch(current, () => {
  failure.value = null;
});

async function change(): Promise<void> {
  if (!canSubmit.value || busy.value) {
    return;
  }
  busy.value = true;
  changed.value = false;
  failure.value = null;

  const outcome = await vault.changePassword(current.value, next.value);
  if (outcome.ok) {
    changed.value = true;
    forget();
  } else {
    failure.value = describeShellError(translate, outcome.failure);
  }
  busy.value = false;
}

/** Locks the vault. What comes next is the gate, which replaces this page. */
async function lockNow(): Promise<void> {
  if (busy.value) {
    return;
  }
  busy.value = true;
  await vault.lock();
  busy.value = false;
}
</script>

<template>
  <div class="space-y-5">
    <!-- The master password: the one thing about the vault that can be changed. -->
    <section class="space-y-2">
      <h4 class="text-sm font-medium text-muted-foreground">{{ t("settings.security.password.title") }}</h4>
      <div class="rounded-md border border-border bg-muted/20 px-3 py-3">
        <p class="text-[11px] leading-relaxed text-muted-foreground">{{ t("settings.security.password.help") }}</p>

        <form class="mt-3 space-y-3" @submit.prevent="change()">
          <div>
            <label class="block text-xs font-medium" for="vault-current-password">
              {{ t("settings.security.password.current") }}
            </label>
            <input
              id="vault-current-password"
              v-model="current"
              type="password"
              autocomplete="current-password"
              class="mt-1 w-full max-w-sm rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
            />
          </div>

          <div>
            <label class="block text-xs font-medium" for="vault-next-password">
              {{ t("settings.security.password.next") }}
            </label>
            <input
              id="vault-next-password"
              v-model="next"
              type="password"
              autocomplete="new-password"
              class="mt-1 w-full max-w-sm rounded-md border bg-background px-3 py-2 text-sm focus:outline-none"
              :class="tooShort || unchanged ? 'border-destructive' : 'border-border'"
              :placeholder="t('settings.security.password.nextPlaceholder')"
              :aria-invalid="tooShort || unchanged"
            />
            <p v-if="tooShort" class="mt-1 text-[11px] text-destructive">
              {{ t("settings.security.password.tooShort") }}
            </p>
            <p v-else-if="unchanged" class="mt-1 text-[11px] text-destructive">
              {{ t("settings.security.password.same") }}
            </p>
          </div>

          <div>
            <label class="block text-xs font-medium" for="vault-next-confirmation">
              {{ t("settings.security.password.confirm") }}
            </label>
            <input
              id="vault-next-confirmation"
              v-model="confirmation"
              type="password"
              autocomplete="new-password"
              class="mt-1 w-full max-w-sm rounded-md border bg-background px-3 py-2 text-sm focus:outline-none"
              :class="mismatched ? 'border-destructive' : 'border-border'"
              :aria-invalid="mismatched"
            />
            <p v-if="mismatched" class="mt-1 text-[11px] text-destructive">
              {{ t("settings.security.password.mismatch") }}
            </p>
          </div>

          <button
            type="submit"
            class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
            :disabled="!canSubmit || busy"
          >
            <LoaderCircle v-if="busy" :size="12" class="animate-spin" />
            {{ busy ? t("settings.security.password.submitting") : t("settings.security.password.submit") }}
          </button>

          <p v-if="changed" class="text-[11px] text-muted-foreground">
            {{ t("settings.security.password.changed") }}
          </p>
          <p
            v-if="failure"
            role="alert"
            class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
          >
            <span>{{ failure.message }}</span>
            <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ failure.code }}</span>
          </p>
        </form>
      </div>
    </section>

    <!-- Locking, and where the libraries of forgotten passwords went. -->
    <section class="space-y-2">
      <h4 class="text-sm font-medium text-muted-foreground">{{ t("settings.security.vault") }}</h4>

      <div class="flex items-start justify-between gap-4 rounded-md border border-border bg-muted/20 px-3 py-2.5">
        <div class="min-w-0">
          <p class="text-sm font-medium">{{ t("settings.security.lock.title") }}</p>
          <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">
            {{ t("settings.security.lock.help") }}
          </p>
        </div>
        <button
          type="button"
          class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="busy"
          @click="lockNow()"
        >
          <Lock :size="12" />
          {{ t("settings.security.lock.action") }}
        </button>
      </div>

      <div class="flex items-start justify-between gap-4 rounded-md border border-border bg-muted/20 px-3 py-2.5">
        <div class="min-w-0">
          <p class="text-sm font-medium">{{ t("settings.security.archives.title") }}</p>
          <p class="mt-1 text-[11px] leading-relaxed text-muted-foreground">
            {{ t("settings.security.archives.help") }}
          </p>
        </div>
        <button
          type="button"
          class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent"
          @click="historyOpen = true"
        >
          <History :size="12" />
          {{ t("settings.security.archives.action") }}
        </button>
      </div>
    </section>

    <VaultHistoryDialog :open="historyOpen" @update:open="(value: boolean) => (historyOpen = value)" />
  </div>
</template>
