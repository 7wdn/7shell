<script setup lang="ts">
/**
 * The gate in front of everything that needs a credential.
 *
 * It draws no window chrome of its own: the title bar above it stays where it is, so the window
 * is still draggable and closable while the vault is shut.
 *
 * The two forms are the only two things a locked application can be asked to do. Nothing here
 * offers to "skip" or to remember the password: the backend has no way to accept either, and a
 * screen that pretended otherwise would be lying about what protects the credentials.
 *
 * Under them, two ways out that are not passwords, kept quiet on purpose — a person who knows the
 * password should not have their eye caught by the exits:
 *
 * * **Archive and reset** is for a password that is gone. It renames the file beside itself rather
 *   than deleting anything, and it asks once in a dialog rather than twice on the button: what it
 *   costs is a sentence, and a sentence belongs where the second click would have been.
 * * **Archived vaults** is where those renamed files come back into reach — to try a password, to
 *   switch to one, or to read what one holds into the vault that is open.
 *
 * A switch is the one thing on this screen that can end with a *different* file in place, so what
 * comes back from it is said out loud: which library is live now, and that its own password is the
 * one to type.
 */
import { Eye, EyeOff, KeyRound, LoaderCircle, RotateCw, ShieldAlert } from "@lucide/vue";
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import VaultHistoryDialog from "@/components/vault/VaultHistoryDialog.vue";
import { describeShellError } from "@/i18n/errors";
import { MINIMUM_PASSWORD_LENGTH } from "@/lib/vault/password";
import { useVaultStore } from "@/stores/vault";

const { t } = useI18n();
const vault = useVaultStore();

const password = ref("");
const confirmation = ref("");
const passwordField = ref<HTMLInputElement | null>(null);

/**
 * Which of the two fields is being read as text, one switch each.
 *
 * One per field rather than one for the form: they are typed at different moments, and a single
 * switch would reveal the confirmation while somebody is still looking at the password they are
 * choosing. The unlock form's field uses the first of them and the create form both.
 */
const passwordShown = ref(false);
const confirmationShown = ref(false);

/** The archive-and-reset question, and the archived vaults behind the other way out. */
const archiveResetOpen = ref(false);
const historyOpen = ref(false);
/** The library a switch brought in, said once under the form it left behind. */
const switchedTo = ref<string | null>(null);

/** Both are shown while typing rather than on submit, so the rule is visible as it is broken. */
const tooShort = computed(() => password.value.length > 0 && password.value.length < MINIMUM_PASSWORD_LENGTH);
const mismatched = computed(() => confirmation.value.length > 0 && confirmation.value !== password.value);
const canCreate = computed(
  () => password.value.length >= MINIMUM_PASSWORD_LENGTH && password.value === confirmation.value,
);

const problem = computed(() => (vault.failure ? describeShellError(translate, vault.failure) : null));

/**
 * An envelope is shown beside the action that produced it, never in one shared slot at the bottom
 * of the card: a failed unlock belongs under the password field, a failed archive-and-reset inside
 * the question that asked for it. The store records which operation failed so the two can be told
 * apart; the message alone cannot say which button was pressed.
 */
const asideProblem = computed(() =>
  problem.value && vault.failedOperation === "vault_set_aside" ? problem.value : null,
);
const formProblem = computed(() => (problem.value && !asideProblem.value ? problem.value : null));

const heading = computed(() => {
  switch (vault.state) {
    case "missing":
      return t("vault.create.title");
    case "locked":
      return t("vault.unlock.title");
    default:
      return t("vault.checking");
  }
});

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/**
 * Archives the vault nobody can open, and leaves the screen on the create form.
 *
 * The question stays up when it fails: what went wrong is about the action behind it, and the
 * refusal is drawn inside the dialog that asked rather than on the card underneath.
 */
async function archiveAndReset(): Promise<void> {
  if (await vault.setAside()) {
    archiveResetOpen.value = false;
    password.value = "";
    confirmation.value = "";
  }
}

async function create(): Promise<void> {
  if (!canCreate.value) {
    return;
  }
  if (await vault.create(password.value)) {
    password.value = "";
    confirmation.value = "";
    switchedTo.value = null;
  }
}

async function unlock(): Promise<void> {
  await vault.unlock(password.value);
}

/** A switch replaced the file this screen is about: say which one is live before the field is typed in. */
function vaultSwitch(name: string): void {
  switchedTo.value = name;
  password.value = "";
}

// The field is focused when a form appears, not at mount: at mount the backend has not said which
// form this is, and focusing a field that is about to be replaced would put the caret nowhere.
watch(
  () => vault.state,
  async (state) => {
    if (state === "missing" || state === "locked") {
      await nextTick();
      passwordField.value?.focus();
    }
  },
  { immediate: true },
);

/**
 * A refused password puts the caret back in the field with everything selected, so retrying is one
 * keystroke rather than a trip to the mouse. Only an unlock does this: the other failures on this
 * screen are not about what was typed, and stealing focus for them would be noise.
 */
watch(
  () => vault.failure,
  async (failure) => {
    if (!failure || vault.failedOperation !== "vault_unlock") {
      return;
    }
    await nextTick();
    passwordField.value?.focus();
    passwordField.value?.select();
  },
);

/**
 * The refusal goes away when what it was about is edited.
 *
 * "The password is wrong" is a sentence about the value in that field, so it stops being true the
 * moment the value changes — and a red border left standing over a field somebody has already
 * corrected is the screen arguing with itself. Only an unlock is dropped this way: a create that
 * failed on the disk is still true after a keystroke, and the store records which operation the
 * message belongs to, so the two can be told apart here.
 */
watch(password, () => {
  if (vault.failedOperation === "vault_unlock") {
    vault.dismissFailure();
  }
});
</script>

<template>
  <div class="flex min-h-0 flex-1 items-center justify-center overflow-auto p-6">
    <div class="w-full max-w-md rounded-lg border border-border bg-card p-6">
      <div class="flex items-center gap-3">
        <span class="grid size-9 shrink-0 place-items-center rounded-md border border-border bg-background">
          <ShieldAlert v-if="problem" :size="16" class="text-destructive" />
          <KeyRound v-else :size="16" class="text-muted-foreground" />
        </span>
        <h1 class="text-sm font-medium">{{ heading }}</h1>
      </div>

      <template v-if="vault.state === 'missing'">
        <p class="mt-4 text-xs leading-relaxed text-muted-foreground">{{ t("vault.create.description") }}</p>

        <form class="mt-4 space-y-3" @submit.prevent="create()">
          <div>
            <label class="block text-xs font-medium" for="vault-password">{{ t("vault.create.password") }}</label>
            <!--
              The eye stays where it is whatever the field is doing: it is the control somebody
              reaches for *after* getting something wrong, and one that vanished into a red border
              would take away exactly the help it exists to give.
            -->
            <div class="relative mt-1">
              <input
                id="vault-password"
                ref="passwordField"
                v-model="password"
                :type="passwordShown ? 'text' : 'password'"
                autocomplete="new-password"
                class="w-full rounded-md border bg-background px-3 py-2 pr-9 text-sm focus:outline-none"
                :class="tooShort ? 'border-destructive' : 'border-border'"
                :placeholder="t('vault.create.passwordPlaceholder')"
                :aria-invalid="tooShort"
                :aria-describedby="tooShort ? 'vault-create-password-error' : undefined"
              />
              <button
                type="button"
                class="absolute inset-y-0 right-0 grid w-8 place-items-center text-muted-foreground transition-colors hover:text-foreground"
                :aria-label="passwordShown ? t('vault.hidePassword') : t('vault.showPassword')"
                :aria-pressed="passwordShown"
                @click="passwordShown = !passwordShown"
              >
                <EyeOff v-if="passwordShown" :size="13" />
                <Eye v-else :size="13" />
              </button>
            </div>
            <p id="vault-create-password-error" v-if="tooShort" class="mt-1 text-[11px] text-destructive">
              {{ t("vault.create.tooShort") }}
            </p>
          </div>

          <div>
            <label class="block text-xs font-medium" for="vault-confirmation">{{ t("vault.create.confirm") }}</label>
            <div class="relative mt-1">
              <input
                id="vault-confirmation"
                v-model="confirmation"
                :type="confirmationShown ? 'text' : 'password'"
                autocomplete="new-password"
                class="w-full rounded-md border bg-background px-3 py-2 pr-9 text-sm focus:outline-none"
                :class="mismatched ? 'border-destructive' : 'border-border'"
                :placeholder="t('vault.create.confirmPlaceholder')"
                :aria-invalid="mismatched"
                :aria-describedby="mismatched ? 'vault-create-confirm-error' : undefined"
              />
              <button
                type="button"
                class="absolute inset-y-0 right-0 grid w-8 place-items-center text-muted-foreground transition-colors hover:text-foreground"
                :aria-label="confirmationShown ? t('vault.hidePassword') : t('vault.showPassword')"
                :aria-pressed="confirmationShown"
                @click="confirmationShown = !confirmationShown"
              >
                <EyeOff v-if="confirmationShown" :size="13" />
                <Eye v-else :size="13" />
              </button>
            </div>
            <p id="vault-create-confirm-error" v-if="mismatched" class="mt-1 text-[11px] text-destructive">
              {{ t("vault.create.mismatch") }}
            </p>
          </div>

          <button
            type="submit"
            class="flex w-full items-center justify-center gap-1.5 rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
            :disabled="!canCreate || vault.busy"
          >
            <LoaderCircle v-if="vault.busy" :size="14" class="animate-spin" />
            {{ vault.busy ? t("vault.create.submitBusy") : t("vault.create.submit") }}
          </button>

          <p
            id="vault-create-error"
            v-if="formProblem"
            role="alert"
            class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
          >
            <span>{{ formProblem.message }}</span>
            <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ formProblem.code }}</span>
          </p>
        </form>
      </template>

      <template v-else-if="vault.state === 'locked'">
        <p class="mt-4 text-xs leading-relaxed text-muted-foreground">{{ t("vault.unlock.description") }}</p>

        <form class="mt-4 space-y-3" @submit.prevent="unlock()">
          <div>
            <label class="block text-xs font-medium" for="vault-password">{{ t("vault.unlock.password") }}</label>
            <div class="relative mt-1">
              <input
                id="vault-password"
                ref="passwordField"
                v-model="password"
                :type="passwordShown ? 'text' : 'password'"
                autocomplete="current-password"
                class="w-full rounded-md border bg-background px-3 py-2 pr-9 text-sm focus:outline-none"
                :class="formProblem ? 'border-destructive' : 'border-border'"
                :placeholder="t('vault.unlock.passwordPlaceholder')"
                :aria-invalid="formProblem ? true : undefined"
                :aria-describedby="formProblem ? 'vault-unlock-error' : undefined"
              />
              <!--
                Always drawn, and the refusal below changes nothing about it: the moment somebody
                most wants to check what they typed is the moment the field turned red, and an eye
                that disappeared there would be missing exactly when it is worth having.
              -->
              <button
                type="button"
                data-vault-password-eye
                class="absolute inset-y-0 right-0 grid w-8 place-items-center text-muted-foreground transition-colors hover:text-foreground"
                :aria-label="passwordShown ? t('vault.hidePassword') : t('vault.showPassword')"
                :aria-pressed="passwordShown"
                @click="passwordShown = !passwordShown"
              >
                <EyeOff v-if="passwordShown" :size="13" />
                <Eye v-else :size="13" />
              </button>
            </div>

            <!--
              The refusal sits under the field it is about. At the bottom of the card it arrived after
              the ways out had already offered the next thing to click, which put a destructive action
              in the path of somebody who had only mistyped.
            -->
            <p
              id="vault-unlock-error"
              v-if="formProblem"
              role="alert"
              class="mt-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
            >
              <span>{{ formProblem.message }}</span>
              <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ formProblem.code }}</span>
            </p>
          </div>

          <button
            type="submit"
            class="flex w-full items-center justify-center gap-1.5 rounded-md bg-primary px-3 py-2 text-sm text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
            :disabled="password.length === 0 || vault.busy"
          >
            <LoaderCircle v-if="vault.busy" :size="14" class="animate-spin" />
            {{ vault.busy ? t("vault.unlock.submitBusy") : t("vault.unlock.submit") }}
          </button>
        </form>
      </template>

      <template v-else-if="problem">
        <p class="mt-4 text-sm leading-relaxed text-muted-foreground">{{ t("vault.unavailable") }}</p>
        <button
          type="button"
          class="mt-4 flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent"
          @click="vault.initialize()"
        >
          <RotateCw :size="12" />
          {{ t("common.action.retry") }}
        </button>

        <p
          id="vault-state-error"
          v-if="problem"
          role="alert"
          class="mt-4 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
        >
          <span>{{ problem.message }}</span>
          <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ problem.code }}</span>
        </p>
      </template>

      <p v-else class="mt-4 flex items-center gap-2 text-sm text-muted-foreground">
        <LoaderCircle :size="14" class="animate-spin" />
        {{ t("vault.checking") }}
      </p>

      <!--
        What there is to do when the password is the problem, or when a library was put aside and is
        wanted back. Drawn in the same quiet weight for both: neither is the ordinary way into the
        application, and neither should catch the eye of somebody who knows their password.

        Each is a button the size of its own words rather than of the row. A block across the card
        is a target somebody hits while aiming at nothing — the empty half of the line is not what
        the words say — and the padding is there only to make the words comfortable to hit.
      -->
      <div
        v-if="vault.state === 'missing' || vault.state === 'locked'"
        class="mt-5 flex flex-col items-center gap-1.5 border-t border-border pt-3"
      >
        <p v-if="vault.setAsideName" class="text-[11px] leading-relaxed text-muted-foreground">
          {{ t("vault.setAside.moved", { name: vault.setAsideName }) }}
        </p>
        <p v-if="switchedTo" class="text-[11px] leading-relaxed text-muted-foreground">
          {{ t("vault.history.activated", { name: switchedTo }) }}
        </p>

        <button
          v-if="vault.state === 'locked'"
          type="button"
          class="rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="archiveResetOpen = true"
        >
          {{ t("vault.reset.link") }}
        </button>
        <button
          type="button"
          class="rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="historyOpen = true"
        >
          {{ t("vault.history.link") }}
        </button>
      </div>
    </div>

    <!--
      The one action on this screen that changes what is on disk, asked about once and in full: the
      consequence, the fact that nothing is deleted, and a button that says what it does.
    -->
    <ConfirmDialog
      :open="archiveResetOpen"
      :title="t('vault.reset.title')"
      :message="t('vault.reset.message')"
      :confirm-label="t('vault.reset.confirm')"
      danger
      :loading="vault.busy"
      @update:open="(value: boolean) => (archiveResetOpen = value)"
      @confirm="archiveAndReset()"
    >
      <p
        v-if="asideProblem"
        role="alert"
        class="mt-3 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
      >
        <span>{{ asideProblem.message }}</span>
        <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ asideProblem.code }}</span>
      </p>
    </ConfirmDialog>

    <VaultHistoryDialog
      :open="historyOpen"
      @update:open="(value: boolean) => (historyOpen = value)"
      @activated="vaultSwitch"
    />
  </div>
</template>
