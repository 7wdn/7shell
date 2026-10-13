<script setup lang="ts">
/**
 * The libraries kept beside the live one, and the three things somebody can want from one.
 *
 * A vault that cannot be opened is renamed rather than deleted — the unlock screen offers that much
 * — and this is where those files come back into reach. It is drawn as a dialog rather than a page
 * because everything it does ends at the same place: the vault, which is what the gate behind it is
 * about, and a page would put a second screen between somebody and the password they came back to
 * type.
 *
 * Three rules, and they are about what a person is told before they commit to anything:
 *
 * 1. **Looking is not switching.** Trying a password opens the archive and drops it, and the live
 *    vault is untouched either way; what a refusal means is shown under the field it was typed in,
 *    which is where somebody who mistyped is already looking.
 * 2. **What arrives is added, never swapped in.** Importing goes through the rules an imported file
 *    follows — nothing already here is replaced — and the counts that come back are said out loud,
 *    including a key this device does not hold.
 * 3. **A switch says what it costs first.** It is the one action here that changes which file the
 *    application answers from, so the row opens into the consequence spelled out — the library in
 *    use is archived, not deleted — with the two answers named.
 *
 * Importing needs a vault that is open and switching needs one that is shut, because those are what
 * the two acts are: adding to a library, and replacing which library is the one being used. A
 * control that cannot be pressed says which of those is missing rather than failing when pressed.
 */
import { Archive, Check, Eye, EyeOff, FolderOpen, LoaderCircle, X } from "@lucide/vue";
import {
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogOverlay,
  DialogPortal,
  DialogRoot,
  DialogTitle,
} from "reka-ui";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import { formatModified, formatSize } from "@/lib/files/format";
import type { ShellError } from "@/lib/backend/errors";
import { useVaultStore } from "@/stores/vault";
import type { SessionImportSummary, VaultArchive } from "@/types/session";

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ "update:open": [value: boolean]; activated: [name: string] }>();

const { t } = useI18n();
const vault = useVaultStore();

/** The row whose password field is open, and what was typed into it. One at a time. */
const trying = ref<string | null>(null);
const password = ref("");
/** Whether that password is drawn as text. Reset whenever the field is opened again. */
const passwordShown = ref(false);
/** The row whose password opened, which is what makes importing this one possible. */
const opened = ref<string | null>(null);
const tryFailure = ref<ShellError | null>(null);
/** What an import wrote, for the row that asked for it. */
const imported = ref<{ name: string; summary: SessionImportSummary } | null>(null);
const importFailure = ref<ShellError | null>(null);
/** The row a switch was asked for, waiting for its second answer, and how that went. */
const confirming = ref<string | null>(null);
const switchFailure = ref<ShellError | null>(null);
/** Whether something is running, so no second control is pressed under the first one. */
const busy = ref(false);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const listProblem = computed(() =>
  vault.archivesFailure ? describeShellError(translate, vault.archivesFailure) : null,
);

/**
 * What the header says, which is the sentence that is true of what is on screen.
 *
 * The "nothing here yet" one is only said once a look has *finished* and found none: while the
 * directory is being read the honest answer is still the general one, because "there are no
 * archives" and "nobody has looked yet" are different claims and only one of them is known.
 */
const description = computed(() => {
  const nothingFound = !vault.archivesBusy && !vault.archivesFailure && vault.archives.length === 0;
  return nothingFound ? t("vault.history.empty") : t("vault.history.description");
});
const tryProblem = computed(() => (tryFailure.value ? describeShellError(translate, tryFailure.value) : null));
const importProblem = computed(() => (importFailure.value ? describeShellError(translate, importFailure.value) : null));
const switchProblem = computed(() => (switchFailure.value ? describeShellError(translate, switchFailure.value) : null));

/** The extra lines an import's counts earn: what could not be carried is said rather than hidden. */
const importNotes = computed(() => {
  const summary = imported.value?.summary;
  if (!summary) {
    return [];
  }
  const notes: string[] = [];
  if (summary.missingKeys > 0) {
    notes.push(t("vault.history.importedMissingKeys", { count: String(summary.missingKeys) }));
  }
  if (summary.droppedHops > 0) {
    notes.push(t("vault.history.importedDroppedHops", { count: String(summary.droppedHops) }));
  }
  return notes;
});

/**
 * Asking again on every open, and starting from a clean slate.
 *
 * The list is a directory listing, which changes while this is shut — a vault moved aside by the
 * gate behind this dialog is the ordinary case — so what is drawn is what a look right now found.
 * The passwords of the previous visit go too: they were typed for rows that may not be there.
 */
watch(
  () => props.open,
  (open) => {
    if (!open) {
      return;
    }
    trying.value = null;
    password.value = "";
    passwordShown.value = false;
    opened.value = null;
    tryFailure.value = null;
    imported.value = null;
    importFailure.value = null;
    confirming.value = null;
    switchFailure.value = null;
    void vault.loadArchives();
  },
  { immediate: true },
);

function when(archive: VaultArchive): string {
  return archive.modifiedAt > 0 ? formatModified(archive.modifiedAt) : t("vault.history.unknownTime");
}

/**
 * Opens or closes one row's password field, forgetting whatever the last one concluded.
 *
 * One panel per card at a time, and it is the same rule as the switch below: a card that grew both
 * of them at once is a card taller than the list it belongs to, and the box somebody is looking for
 * ends up under a scrollbar rather than under their eye.
 */
function toggleTry(archive: VaultArchive): void {
  trying.value = trying.value === archive.name ? null : archive.name;
  password.value = "";
  passwordShown.value = false;
  opened.value = null;
  tryFailure.value = null;
  imported.value = null;
  importFailure.value = null;
  confirming.value = null;
}

/**
 * Checks the typed password. Nothing about the live vault changes here, so it can be repeated as
 * often as somebody mistypes — which is the whole reason a try and a switch are two acts.
 */
async function check(archive: VaultArchive): Promise<void> {
  if (busy.value || password.value.length === 0) {
    return;
  }
  busy.value = true;
  tryFailure.value = null;
  imported.value = null;

  const outcome = await vault.verifyArchive(archive.name, password.value);
  if (outcome.ok) {
    opened.value = archive.name;
  } else {
    opened.value = null;
    tryFailure.value = outcome.failure;
  }
  busy.value = false;
}

/**
 * Adds what the archive holds to the vault that is open. The password is the one just checked.
 *
 * It refuses while the vault is shut, and the condition is the same one the control's
 * `aria-disabled` carries: a control that only *looks* unavailable has to be able to say no, or the
 * tooltip explaining it becomes decoration.
 */
async function bringIn(archive: VaultArchive): Promise<void> {
  if (busy.value || !vault.unlocked) {
    return;
  }
  busy.value = true;
  importFailure.value = null;

  const outcome = await vault.importArchive(archive.name, password.value);
  if (outcome.ok) {
    imported.value = { name: archive.name, summary: outcome.value };
  } else {
    importFailure.value = outcome.failure;
  }
  busy.value = false;
}

/**
 * Editing the password takes back everything that was said about the old one.
 *
 * A refusal is about what was typed and a check is a fact about the same value, so neither survives
 * a keystroke: the refusal would be the screen arguing with a value that has changed, and the import
 * button would offer to carry a library over with a password nobody verified — the check is not
 * re-run behind it, the value it was run for is simply gone.
 */
watch(password, () => {
  tryFailure.value = null;
  importFailure.value = null;
  opened.value = null;
});

/**
 * Opens the row's second question: what a switch does, in full, with both answers named.
 *
 * It puts the password field away on the way, and that is the mutual exclusion rather than
 * tidiness: the two are answers to different questions about the same file — *can this open?* and
 * *should this take the live vault's place?* — and a card holding both is a card somebody has to
 * scroll to read either of.
 */
function askToSwitch(archive: VaultArchive): void {
  if (busy.value || vault.unlocked) {
    return;
  }

  trying.value = null;
  password.value = "";
  passwordShown.value = false;
  opened.value = null;
  tryFailure.value = null;
  imported.value = null;
  importFailure.value = null;

  confirming.value = archive.name;
  switchFailure.value = null;
}

/**
 * Makes the archive the live vault, and closes the dialog on the way.
 *
 * Closed because what happens next is the gate's: the file the application answers from is not the
 * one it was, so the password field behind this dialog is the next thing to use. The name of the
 * library that was archived goes up with the event, for whoever wants to say where it went.
 */
async function switchTo(archive: VaultArchive): Promise<void> {
  if (busy.value) {
    return;
  }
  busy.value = true;
  switchFailure.value = null;

  const outcome = await vault.activateArchive(archive.name);
  busy.value = false;
  if (outcome.ok) {
    confirming.value = null;
    emit("activated", archive.name);
    emit("update:open", false);
    return;
  }
  switchFailure.value = outcome.failure;
}

async function reveal(): Promise<void> {
  if (busy.value) {
    return;
  }
  busy.value = true;
  const outcome = await vault.revealFolder();
  if (!outcome.ok) {
    vault.reportArchivesFailure(outcome.failure);
  }
  busy.value = false;
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 flex max-h-[min(38rem,calc(100vh-3rem))] w-[min(36rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 flex-col rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <header class="flex items-start justify-between gap-4 border-b border-border px-4 py-3">
          <div class="min-w-0">
            <DialogTitle class="text-sm font-medium">{{ t("vault.history.title") }}</DialogTitle>
            <!--
              What this dialog is for, said in the sentence that is true of what is on screen: a
              list that found nothing is not a list of archives somebody can switch to, and a header
              claiming "archived vaults were found" over an empty box is the kind of sentence that
              makes a reader doubt the rest of the window.
            -->
            <DialogDescription class="mt-1 text-xs leading-relaxed text-muted-foreground">
              {{ description }}
            </DialogDescription>
          </div>
          <DialogClose
            class="grid size-7 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('vault.history.close')"
          >
            <X :size="15" />
          </DialogClose>
        </header>

        <div class="min-h-0 flex-1 overflow-y-auto px-4 py-3">
          <p v-if="vault.archivesBusy" class="flex items-center gap-2 py-6 text-xs text-muted-foreground">
            <LoaderCircle :size="13" class="animate-spin" />
            {{ t("vault.history.loading") }}
          </p>

          <p
            v-else-if="listProblem"
            role="alert"
            class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
          >
            <span>{{ listProblem.message }}</span>
            <span class="mt-1 block font-mono text-[11px] text-muted-foreground">{{ listProblem.code }}</span>
          </p>

          <ul v-else class="space-y-2">
            <li
              v-for="archive in vault.archives"
              :key="archive.name"
              class="rounded-md border border-border px-3 py-2.5"
            >
              <div class="flex items-start gap-2.5">
                <span
                  class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border border-border bg-background"
                >
                  <Archive :size="13" class="text-muted-foreground" />
                </span>
                <div class="min-w-0 flex-1">
                  <p class="font-mono text-xs break-all">{{ archive.name }}</p>
                  <p class="mt-1 text-[11px] text-muted-foreground">
                    {{ when(archive) }} · {{ formatSize(archive.sizeBytes) }}
                  </p>

                  <div class="mt-2 flex flex-wrap gap-1.5">
                    <!--
                      Two of the three controls here can be unavailable, and each carries
                      `aria-disabled` rather than `disabled` with its reason drawn as a tooltip.
                      Neither half of that is a preference:

                      * A control with the `disabled` attribute is skipped by hit testing, so nothing
                        that hovers it ever fires — the sentence would be a tooltip that cannot open.
                        The click handlers refuse on the same condition, so the controls are still
                        unusable; `group-focus-within` is what makes the reason reachable without a
                        mouse, which a `disabled` control can never be.
                      * The tooltip is drawn in the dialog rather than by the page's tooltip
                        component, because that one renders into a portal on the body and this dialog
                        is modal: everything outside it — the portal included — is `aria-hidden`
                        while it is open, so the sentence would be drawn and then hidden from the
                        one reader who needs it. (That was found by the end-to-end run, which asked
                        for the sentence and got an empty answer.)
                    -->
                    <button
                      type="button"
                      class="rounded-md border border-border px-2.5 py-1 text-[11px] transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
                      :disabled="busy"
                      :aria-expanded="trying === archive.name"
                      @click="toggleTry(archive)"
                    >
                      {{ t("vault.history.test") }}
                    </button>
                    <span class="group relative inline-flex">
                      <button
                        type="button"
                        class="rounded-md border border-border px-2.5 py-1 text-[11px] transition-colors"
                        :class="busy || vault.unlocked ? 'cursor-not-allowed opacity-50' : 'hover:bg-accent'"
                        :aria-disabled="busy || vault.unlocked"
                        :aria-describedby="vault.unlocked ? `vault-switch-reason-${archive.name}` : undefined"
                        @click="askToSwitch(archive)"
                      >
                        {{ t("vault.history.activate") }}
                      </button>
                      <span
                        v-if="vault.unlocked"
                        :id="`vault-switch-reason-${archive.name}`"
                        role="tooltip"
                        class="pointer-events-none absolute top-full left-0 z-10 mt-1 hidden w-max max-w-64 rounded-md border border-border bg-card px-2.5 py-1.5 text-[11px] leading-relaxed text-muted-foreground shadow-md group-hover:block group-focus-within:block"
                      >
                        {{ t("vault.history.activateNeedsLock") }}
                      </span>
                    </span>
                  </div>

                  <!-- Trying a password: the refusal belongs under the field it was typed in. -->
                  <form
                    v-if="trying === archive.name"
                    class="mt-2.5 space-y-2 rounded-md border border-border bg-muted/30 px-2.5 py-2"
                    @submit.prevent="check(archive)"
                  >
                    <label class="block text-[11px] font-medium" :for="`vault-archive-password-${archive.name}`">
                      {{ t("vault.unlock.password") }}
                    </label>
                    <!--
                      The eye is the same control the sync page draws, and the button next to it is
                      the submit — so Enter in the field verifies, which is what a field with one
                      button under it invites. `type="button"` on the eye is what keeps it from
                      being the button Enter presses.
                    -->
                    <div class="relative">
                      <input
                        :id="`vault-archive-password-${archive.name}`"
                        v-model="password"
                        :type="passwordShown ? 'text' : 'password'"
                        autocomplete="current-password"
                        class="w-full rounded-md border bg-background px-2.5 py-1.5 pr-8 text-xs focus:outline-none"
                        :class="tryProblem ? 'border-destructive' : 'border-border'"
                        :placeholder="t('vault.history.passwordPlaceholder')"
                        :aria-invalid="tryProblem ? true : undefined"
                        :aria-describedby="tryProblem ? `vault-archive-error-${archive.name}` : undefined"
                      />
                      <button
                        type="button"
                        class="absolute inset-y-0 right-0 grid w-7 place-items-center text-muted-foreground transition-colors hover:text-foreground"
                        :aria-label="passwordShown ? t('vault.hidePassword') : t('vault.showPassword')"
                        @click="passwordShown = !passwordShown"
                      >
                        <EyeOff v-if="passwordShown" :size="12" />
                        <Eye v-else :size="12" />
                      </button>
                    </div>
                    <div class="flex flex-wrap items-center gap-1.5">
                      <button
                        type="submit"
                        class="rounded-md bg-primary px-2.5 py-1 text-[11px] text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
                        :disabled="busy || password.length === 0"
                      >
                        {{ busy ? t("vault.history.verifying") : t("vault.history.verify") }}
                      </button>
                      <!--
                        Once it has been read in, the button is gone and the counts stand in its
                        place: an import adds rather than replaces, so a second press would be a
                        second copy of the same library — which is a thing to ask for, not a thing
                        to leave one click away.

                        Unavailable while the vault is shut, with the reason on hover and on focus —
                        see the note over the row's controls for why that is `aria-disabled` and a
                        tooltip drawn here rather than the page's tooltip component.
                      -->
                      <span
                        v-if="opened === archive.name && imported?.name !== archive.name"
                        class="group relative inline-flex"
                      >
                        <button
                          type="button"
                          class="rounded-md border border-border px-2.5 py-1 text-[11px] transition-colors"
                          :class="busy || !vault.unlocked ? 'cursor-not-allowed opacity-50' : 'hover:bg-accent'"
                          :aria-disabled="busy || !vault.unlocked"
                          :aria-describedby="vault.unlocked ? undefined : `vault-import-reason-${archive.name}`"
                          @click="bringIn(archive)"
                        >
                          {{ busy ? t("vault.history.importing") : t("vault.history.import") }}
                        </button>
                        <span
                          v-if="!vault.unlocked"
                          :id="`vault-import-reason-${archive.name}`"
                          role="tooltip"
                          class="pointer-events-none absolute top-full left-0 z-10 mt-1 hidden w-max max-w-64 rounded-md border border-border bg-card px-2.5 py-1.5 text-[11px] leading-relaxed text-muted-foreground shadow-md group-hover:block group-focus-within:block"
                        >
                          {{ t("vault.history.importNeedsUnlock") }}
                        </span>
                      </span>
                    </div>

                    <p
                      v-if="tryProblem"
                      :id="`vault-archive-error-${archive.name}`"
                      role="alert"
                      class="rounded-md border border-destructive/40 bg-destructive/10 px-2.5 py-1.5 text-[11px]"
                    >
                      <span>{{ tryProblem.message }}</span>
                      <span class="mt-0.5 block font-mono text-[11px] text-muted-foreground">{{
                        tryProblem.code
                      }}</span>
                    </p>
                    <p
                      v-else-if="opened === archive.name"
                      class="flex items-center gap-1.5 text-[11px] text-muted-foreground"
                    >
                      <Check :size="12" />
                      {{ t("vault.history.verified") }}
                    </p>
                  </form>

                  <!-- What an import wrote, counted the way an imported file is counted. -->
                  <div
                    v-if="imported?.name === archive.name"
                    class="mt-2 space-y-0.5 rounded-md border border-border bg-muted/30 px-2.5 py-2 text-[11px]"
                  >
                    <p>
                      {{
                        t("vault.history.imported", {
                          sessions: String(imported.summary.sessions),
                          groups: String(imported.summary.groups),
                          secrets: String(imported.summary.secrets),
                        })
                      }}
                    </p>
                    <p v-for="note in importNotes" :key="note" class="text-muted-foreground">{{ note }}</p>
                  </div>
                  <p
                    v-if="importProblem && trying === archive.name"
                    role="alert"
                    class="mt-2 rounded-md border border-destructive/40 bg-destructive/10 px-2.5 py-1.5 text-[11px]"
                  >
                    <span>{{ importProblem.message }}</span>
                    <span class="mt-0.5 block font-mono text-[11px] text-muted-foreground">{{
                      importProblem.code
                    }}</span>
                  </p>

                  <!-- The switch, with its consequence in front of the two answers. -->
                  <div
                    v-if="confirming === archive.name"
                    class="mt-2.5 space-y-2 rounded-md border border-warning/40 bg-warning/10 px-2.5 py-2"
                  >
                    <p class="text-[11px] font-medium">{{ t("vault.history.activateTitle") }}</p>
                    <p class="text-[11px] leading-relaxed text-muted-foreground">
                      {{ t("vault.history.activateMessage") }}
                    </p>
                    <div class="flex flex-wrap gap-1.5">
                      <button
                        type="button"
                        class="rounded-md border border-border px-2.5 py-1 text-[11px] transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
                        :disabled="busy"
                        @click="confirming = null"
                      >
                        {{ t("common.action.cancel") }}
                      </button>
                      <button
                        type="button"
                        class="rounded-md bg-warning/20 px-2.5 py-1 text-[11px] transition-colors hover:bg-warning/30 disabled:cursor-not-allowed disabled:opacity-50"
                        :disabled="busy"
                        @click="switchTo(archive)"
                      >
                        {{ t("vault.history.activateConfirm") }}
                      </button>
                    </div>
                  </div>

                  <p
                    v-if="switchProblem && confirming === archive.name"
                    role="alert"
                    class="mt-2 rounded-md border border-destructive/40 bg-destructive/10 px-2.5 py-1.5 text-[11px]"
                  >
                    <span>{{ switchProblem.message }}</span>
                    <span class="mt-0.5 block font-mono text-[11px] text-muted-foreground">{{
                      switchProblem.code
                    }}</span>
                  </p>
                </div>
              </div>
            </li>
          </ul>
        </div>

        <footer
          class="flex items-center justify-between gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3"
        >
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-50"
            :disabled="busy"
            @click="reveal()"
          >
            <FolderOpen :size="12" />
            {{ t("vault.history.openFolder") }}
          </button>
          <DialogClose class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent">
            {{ t("vault.history.close") }}
          </DialogClose>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
