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
 */
import { KeyRound, LoaderCircle, RotateCw, ShieldAlert } from "@lucide/vue";
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { useVaultStore } from "@/stores/vault";

/** The shortest password the create form accepts. */
const MINIMUM_PASSWORD_LENGTH = 8;

const { t } = useI18n();
const vault = useVaultStore();

const password = ref("");
const confirmation = ref("");
const passwordField = ref<HTMLInputElement | null>(null);

/** The set-aside action asks twice: it is the only thing here that changes what is on disk. */
const confirmingAside = ref(false);

/** Both are shown while typing rather than on submit, so the rule is visible as it is broken. */
const tooShort = computed(() => password.value.length > 0 && password.value.length < MINIMUM_PASSWORD_LENGTH);
const mismatched = computed(() => confirmation.value.length > 0 && confirmation.value !== password.value);
const canCreate = computed(
  () => password.value.length >= MINIMUM_PASSWORD_LENGTH && password.value === confirmation.value,
);

const problem = computed(() => (vault.failure ? describeShellError(translate, vault.failure) : null));

/**
 * An envelope is shown beside the action that produced it, never in one shared slot at the bottom
 * of the card: a failed unlock belongs under the password field, a failed set-aside under the
 * button that tried to rename the file. The store records which operation failed so the two can be
 * told apart; the message alone cannot say which button was pressed.
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
 * Moves an unopenable vault aside on the second click. The first click only arms the button, so a
 * stray Enter key cannot rename somebody's file.
 */
async function setAside(): Promise<void> {
  if (!confirmingAside.value) {
    confirmingAside.value = true;
    return;
  }

  await vault.setAside();
  confirmingAside.value = false;
}

async function create(): Promise<void> {
  if (!canCreate.value) {
    return;
  }
  if (await vault.create(password.value)) {
    password.value = "";
    confirmation.value = "";
  }
}

async function unlock(): Promise<void> {
  await vault.unlock(password.value);
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
            <input
              id="vault-password"
              ref="passwordField"
              v-model="password"
              type="password"
              autocomplete="new-password"
              class="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm focus:outline-none"
              :class="tooShort ? 'border-destructive' : 'border-border'"
              :placeholder="t('vault.create.passwordPlaceholder')"
              :aria-invalid="tooShort"
              :aria-describedby="tooShort ? 'vault-create-password-error' : undefined"
            />
            <p id="vault-create-password-error" v-if="tooShort" class="mt-1 text-[11px] text-destructive">
              {{ t("vault.create.tooShort") }}
            </p>
          </div>

          <div>
            <label class="block text-xs font-medium" for="vault-confirmation">{{ t("vault.create.confirm") }}</label>
            <input
              id="vault-confirmation"
              v-model="confirmation"
              type="password"
              autocomplete="new-password"
              class="mt-1 w-full rounded-md border bg-background px-3 py-2 text-sm focus:outline-none"
              :class="mismatched ? 'border-destructive' : 'border-border'"
              :placeholder="t('vault.create.confirmPlaceholder')"
              :aria-invalid="mismatched"
              :aria-describedby="mismatched ? 'vault-create-confirm-error' : undefined"
            />
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
            <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ formProblem.code }}</span>
          </p>
        </form>
      </template>

      <template v-else-if="vault.state === 'locked'">
        <p class="mt-4 text-xs leading-relaxed text-muted-foreground">{{ t("vault.unlock.description") }}</p>

        <form class="mt-4 space-y-3" @submit.prevent="unlock()">
          <div>
            <label class="block text-xs font-medium" for="vault-password">{{ t("vault.unlock.password") }}</label>
            <input
              id="vault-password"
              ref="passwordField"
              v-model="password"
              type="password"
              autocomplete="current-password"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
              :class="formProblem ? 'border-destructive' : 'border-border'"
              :placeholder="t('vault.unlock.passwordPlaceholder')"
              :aria-invalid="formProblem ? true : undefined"
              :aria-describedby="formProblem ? 'vault-unlock-error' : undefined"
            />

            <!--
              The refusal sits under the field it is about. At the bottom of the card it arrived
              after the fallback block had already offered the next thing to click, which put the
              destructive action in the path of somebody who had only mistyped.
            -->
            <p
              id="vault-unlock-error"
              v-if="formProblem"
              role="alert"
              class="mt-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
            >
              <span>{{ formProblem.message }}</span>
              <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ formProblem.code }}</span>
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

        <!--
          A password nobody remembers must not be a dead end. This renames the file beside itself
          rather than deleting anything, and asks twice, because it is the one action here that
          changes what is on disk.
        -->
        <div class="mt-5 rounded-md border border-dashed border-border px-3 py-3">
          <p class="text-xs leading-relaxed text-muted-foreground">{{ t("vault.setAside.explanation") }}</p>

          <p v-if="vault.setAsideName" class="mt-2 text-xs">
            {{ t("vault.setAside.moved", { name: vault.setAsideName }) }}
          </p>

          <button
            v-else
            type="button"
            class="mt-2 w-full rounded-md border px-3 py-1.5 text-xs transition-colors disabled:opacity-50"
            :class="
              confirmingAside
                ? 'border-destructive/50 bg-destructive/10 text-destructive'
                : 'border-border hover:bg-accent'
            "
            :disabled="vault.busy"
            @click="setAside()"
          >
            {{ confirmingAside ? t("vault.setAside.confirm") : t("vault.setAside.submit") }}
          </button>

          <p
            id="vault-aside-error"
            v-if="asideProblem"
            role="alert"
            class="mt-2 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
          >
            <span>{{ asideProblem.message }}</span>
            <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ asideProblem.code }}</span>
          </p>
        </div>
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
          <span class="mt-1 block font-mono text-[10px] text-muted-foreground">{{ problem.code }}</span>
        </p>
      </template>

      <p v-else class="mt-4 flex items-center gap-2 text-sm text-muted-foreground">
        <LoaderCircle :size="14" class="animate-spin" />
        {{ t("vault.checking") }}
      </p>
    </div>
  </div>
</template>
