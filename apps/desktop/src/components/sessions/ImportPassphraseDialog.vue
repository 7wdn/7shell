<script setup lang="ts">
/**
 * The question a sealed export asks before it can be read: what is it sealed with?
 *
 * The file is already chosen by the time this opens — it had to be read to learn that it is sealed
 * — so the question says which file it is about. A wrong passphrase is said here rather than in the
 * list's failure box, because this is the field it is about and it is still open: the import is
 * parked on the answer, and the answer can be given again.
 *
 * Drawn from the store's question rather than from the component that started the import: the
 * sidebar can be put away while this is on screen, and the home page imports too.
 */
import { DialogContent, DialogDescription, DialogOverlay, DialogPortal, DialogRoot, DialogTitle } from "reka-ui";
import { KeyRound, LoaderCircle } from "@lucide/vue";
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";

import { describeShellError } from "@/i18n/errors";
import type { ImportQuestion } from "@/stores/sessions";

const props = defineProps<{
  /** What the import stopped on, or nothing when no question is open. */
  question: ImportQuestion | null;
}>();

const emit = defineEmits<{
  /** The passphrase, or `null` for somebody closing the question without one. */
  answer: [passphrase: string | null];
}>();

const { t } = useI18n();

const passphrase = ref("");

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

/** Why the last answer was refused, in the active language. */
const problem = computed(() =>
  props.question?.failure ? describeShellError(translate, props.question.failure).message : "",
);

// Every fresh question starts empty, and a refusal empties it too: the field is about to be typed
// in again, and leaving the rejected passphrase in it only invites pressing Enter on it.
watch(
  () => props.question,
  () => {
    passphrase.value = "";
  },
);

function submit(): void {
  if (!props.question?.busy) {
    emit("answer", passphrase.value);
  }
}
</script>

<template>
  <DialogRoot
    :open="props.question !== null"
    @update:open="(value: boolean) => (value ? undefined : emit('answer', null))"
  >
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(30rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
        @interact-outside="(event: Event) => (props.question?.busy ? event.preventDefault() : undefined)"
        @escape-key-down="(event: Event) => (props.question?.busy ? event.preventDefault() : undefined)"
      >
        <div class="px-4 pt-4">
          <div class="flex items-start gap-2.5">
            <span class="mt-0.5 grid size-7 shrink-0 place-items-center rounded-md border border-border bg-background">
              <KeyRound :size="14" class="text-muted-foreground" />
            </span>
            <div class="min-w-0">
              <DialogTitle class="text-sm font-medium">{{ t("sessions.list.importPassphraseTitle") }}</DialogTitle>
              <DialogDescription class="mt-1 text-xs leading-relaxed text-muted-foreground">
                {{ t("sessions.list.importPassphraseMessage", { file: props.question?.fileName ?? "" }) }}
              </DialogDescription>
            </div>
          </div>

          <div class="mt-3">
            <input
              v-model="passphrase"
              type="password"
              autocomplete="off"
              autofocus
              class="w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:outline-none"
              :aria-label="t('sessions.list.importPassphrase')"
              :placeholder="t('sessions.list.importPassphrasePlaceholder')"
              :disabled="props.question?.busy"
              @keydown.enter="submit()"
            />
            <p v-if="problem" class="mt-1 text-[11px] text-destructive" role="alert">{{ problem }}</p>
          </div>
        </div>

        <footer
          class="mt-4 flex flex-col-reverse gap-2 rounded-b-lg border-t border-border bg-muted/40 px-4 py-3 sm:flex-row sm:justify-end"
        >
          <button
            type="button"
            class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.question?.busy"
            @click="emit('answer', null)"
          >
            {{ t("common.action.cancel") }}
          </button>
          <button
            type="button"
            class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
            :disabled="props.question?.busy"
            @click="submit()"
          >
            <LoaderCircle v-if="props.question?.busy" :size="12" class="mr-1.5 inline animate-spin" />
            {{ t("sessions.list.importPassphraseConfirm") }}
          </button>
        </footer>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
