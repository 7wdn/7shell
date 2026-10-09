<script setup lang="ts">
/**
 * The one question an export asks before it happens: do the passwords go with it?
 *
 * A decision rather than a confirmation, which is why it is not built on `ConfirmDialog`: the two
 * answers are different files for different journeys, not "yes" and "no", and the second one has
 * something to say for itself. The passphrase field *is* the choice — filled in, the stored logins
 * travel sealed inside the file; left empty, the file carries the sessions and groups and not one
 * credential. The shell enforces that split; this only has to make it legible.
 *
 * A passphrase that sealed a file earlier in this session comes back, so exporting twice in one
 * sitting means typing once. It is only ever the passphrase of a write that *succeeded*, and it
 * lives in memory alone — `lib/sessions/exportPassphrase.ts` says what it deliberately leaves out.
 */
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { LoaderCircle, TriangleAlert } from "@lucide/vue";
import { computed, nextTick, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { rememberedExportPassphrase } from "@/lib/sessions/exportPassphrase";

const props = defineProps<{
  open: boolean;
  /** While the write is running: the dialog stays up and nothing can be pressed twice. */
  busy: boolean;
}>();

const emit = defineEmits<{
  /** With the passphrase the file is sealed under. */
  confirm: [passphrase: string];
  /** With nothing: the sessions travel, the logins stay here. */
  unencrypted: [];
  "update:open": [value: boolean];
}>();

const { t } = useI18n();

/**
 * The shortest passphrase the shell will seal a file under.
 *
 * Kept in step with `MIN_PASSPHRASE_LEN` by hand, and checked again on the other side: this is a
 * courtesy that saves a round trip, not the rule, and the rule is the one that counts.
 */
const MIN_PASSPHRASE = 4;

const passphrase = ref("");
const confirmation = ref("");
const problem = ref("");
/** The second question, for the answer that leaves the logins at home. */
const warningOpen = ref(false);
/** Focused with the caret at the end of what is already in it, not at the front. */
const passphraseField = ref<HTMLInputElement | null>(null);

watch(
  () => props.open,
  async (open) => {
    if (!open) {
      return;
    }

    // Opened with what this session last sealed a file under, if it ever did. Both fields, so the
    // confirmation rule is met by the same answer rather than by retyping it.
    const remembered = rememberedExportPassphrase();
    passphrase.value = remembered;
    confirmation.value = remembered;
    problem.value = "";
    warningOpen.value = false;

    // The caret goes to the end of the prefilled text: one put at the front turns "add a word"
    // into "scramble the passphrase".
    await nextTick();
    passphraseField.value?.focus();
    const length = passphraseField.value?.value.length ?? 0;
    passphraseField.value?.setSelectionRange(length, length);
  },
  { immediate: true },
);

/**
 * What is wrong with the passphrase as typed, or an empty string.
 *
 * Counted in code points rather than in UTF-16 units, so a passphrase with an emoji in it is judged
 * here the way the shell judges it.
 */
const complaint = computed(() => {
  if ([...passphrase.value].length < MIN_PASSPHRASE) {
    return t("sessions.list.exportPassphraseShort", { minimum: MIN_PASSPHRASE });
  }
  if (passphrase.value !== confirmation.value) {
    return t("sessions.list.exportPassphraseMismatch");
  }
  return "";
});

/**
 * Sends the passphrase, or says what is wrong with it and stays open. The dialog never closes
 * itself: whether this question should stay on screen depends on what the write does, and the
 * caller is the one that knows — a write that goes ahead closes it, and a refusal leaves it here
 * with the envelope drawn in the list behind.
 */
function submitEncrypted(): void {
  problem.value = complaint.value;
  if (problem.value === "") {
    emit("confirm", passphrase.value);
  }
}
</script>

<template>
  <!--
    One question on screen at a time, the way the frame asks its own: answering "no passphrase" puts
    the first one away rather than stacking a second on top of it, and leaving this one brings it
    back with the fields as they were.
  -->
  <DialogRoot :open="props.open && !warningOpen" @update:open="(value: boolean) => emit('update:open', value)">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <div class="px-4 pt-4">
          <div class="flex items-start gap-2.5">
            <span class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border border-border bg-background">
              <TriangleAlert :size="14" class="text-warning" />
            </span>
            <div class="min-w-0">
              <DialogTitle class="text-sm font-medium">{{ t("sessions.list.exportTitle") }}</DialogTitle>
              <DialogDescription class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t("sessions.list.exportMessage") }}
              </DialogDescription>
            </div>
          </div>

          <div class="mt-3 space-y-2">
            <div>
              <label class="block text-xs" for="export-passphrase">
                {{ t("sessions.list.exportPassphrase") }}
              </label>
              <input
                id="export-passphrase"
                ref="passphraseField"
                v-model="passphrase"
                type="password"
                autocomplete="new-password"
                class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
                :placeholder="t('sessions.list.exportPassphrasePlaceholder')"
                @keydown.enter="submitEncrypted()"
              />
            </div>
            <div>
              <label class="block text-xs" for="export-passphrase-confirm">
                {{ t("sessions.list.exportPassphraseConfirm") }}
              </label>
              <input
                id="export-passphrase-confirm"
                v-model="confirmation"
                type="password"
                autocomplete="new-password"
                class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
                :placeholder="t('sessions.list.exportPassphraseConfirmPlaceholder')"
                @keydown.enter="submitEncrypted()"
              />
            </div>

            <p v-if="problem" class="text-[11px] text-destructive" role="alert">{{ problem }}</p>
            <p v-else class="text-[11px] leading-relaxed text-muted-foreground">
              {{ t("sessions.list.exportPassphraseHint") }}
            </p>
          </div>
        </div>

        <footer
          class="mt-4 flex flex-col-reverse gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3 sm:flex-row sm:justify-end"
        >
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.busy"
            @click="emit('update:open', false)"
          >
            {{ t("common.action.cancel") }}
          </button>
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.busy"
            @click="warningOpen = true"
          >
            {{ t("sessions.list.exportUnencrypted") }}
          </button>
          <button
            type="button"
            class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.busy"
            @click="submitEncrypted()"
          >
            <LoaderCircle v-if="props.busy" :size="12" class="mr-1.5 inline animate-spin" />
            {{ t("sessions.list.exportEncrypted") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>

  <!--
    The other answer, asked once more with what it costs spelled out. Not styled as a danger: the
    file this writes is not a risk to anybody — it simply has no passwords in it, and the person
    taking it somewhere had better know that before they arrive.
  -->
  <ConfirmDialog
    :open="warningOpen"
    :title="t('sessions.list.exportUnencryptedTitle')"
    :message="t('sessions.list.exportUnencryptedMessage')"
    :confirm-label="t('sessions.list.exportUnencrypted')"
    @confirm="
      () => {
        warningOpen = false;
        emit('unencrypted');
      }
    "
    @update:open="warningOpen = $event"
  />
</template>
