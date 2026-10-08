<script setup lang="ts">
/**
 * Writing a word: the command it follows, the word, and what it means.
 *
 * A dialog rather than a row that turns into a form, for the reason the command editor is one: what
 * is being written is a thing of its own, and the list somebody was reading should not rearrange
 * itself under them while they write it.
 *
 * The note is their own sentence rather than a translation, so it is left as typed — the one field
 * here that is not a word of the command line.
 */
import { X } from "@lucide/vue";
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

import { useWordsStore } from "@/stores/words";
import type { CompletionWord } from "@/types/snippet";

const props = defineProps<{
  open: boolean;
  /** The word being changed, or `null` when this is a new one. */
  word: CompletionWord | null;
}>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const words = useWordsStore();

const draft = ref({ id: "", command: "", word: "", note: "" });

// Opening on a word starts from that word; opening for a new one starts empty. Watched rather than
// initialized once because the same dialog is opened again for the next row.
watch(
  () => [props.open, props.word] as const,
  () => {
    if (!props.open) {
      return;
    }
    const word = props.word;
    draft.value = word
      ? { id: word.id, command: word.command, word: word.word, note: word.note }
      : { id: "", command: "", word: "", note: "" };
  },
  { immediate: true },
);

const editing = computed(() => draft.value.id !== "");

/**
 * Both halves are single words, because both are matched against one word.
 *
 * The command line is split into words before anything is looked up, so a command or a word with a
 * space in it could never answer. Said here rather than stored and met later as "it does not work".
 */
const hasSpace = computed(() => /\s/.test(draft.value.command.trim()) || /\s/.test(draft.value.word.trim()));

const canSave = computed(() => draft.value.command.trim() !== "" && draft.value.word.trim() !== "" && !hasSpace.value);

function updateOpen(value: boolean): void {
  emit("update:open", value);
}

async function save(): Promise<void> {
  if (!canSave.value) {
    return;
  }
  if (await words.save(draft.value)) {
    emit("update:open", false);
  }
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="updateOpen">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(34rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <header class="flex items-center justify-between border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">
            {{ editing ? t("settings.words.editTitle") : t("settings.words.addTitle") }}
          </DialogTitle>
          <DialogDescription class="sr-only">{{ t("settings.words.hint") }}</DialogDescription>
          <DialogClose
            class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('common.action.close')"
          >
            <X :size="15" />
          </DialogClose>
        </header>

        <form class="space-y-4 px-4 py-4" @submit.prevent="save()">
          <div>
            <label class="block text-xs font-medium" for="word-command">{{
              t("settings.words.columns.command")
            }}</label>
            <input
              id="word-command"
              v-model="draft.command"
              spellcheck="false"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus:border-primary focus:outline-none"
              :placeholder="t('settings.words.commandPlaceholder')"
            />
          </div>

          <div>
            <label class="block text-xs font-medium" for="word-word">{{ t("settings.words.columns.word") }}</label>
            <input
              id="word-word"
              v-model="draft.word"
              spellcheck="false"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus:border-primary focus:outline-none"
              :placeholder="t('settings.words.wordPlaceholder')"
            />
            <p v-if="hasSpace" class="mt-1 text-xs text-destructive">{{ t("settings.words.spaceInvalid") }}</p>
          </div>

          <div>
            <label class="block text-xs font-medium" for="word-note">{{ t("settings.words.columns.note") }}</label>
            <input
              id="word-note"
              v-model="draft.note"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:border-primary focus:outline-none"
              :placeholder="t('settings.words.notePlaceholder')"
            />
            <p class="mt-1 text-xs text-muted-foreground">{{ t("settings.words.noteHint") }}</p>
          </div>

          <p v-if="words.failure" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
            {{ t("settings.words.saveFailed") }}
          </p>

          <footer class="flex items-center justify-end gap-2 border-t border-border pt-3">
            <DialogClose class="rounded-md border border-border px-3 py-1.5 text-xs transition-colors hover:bg-accent">
              {{ t("common.action.cancel") }}
            </DialogClose>
            <button
              type="submit"
              class="rounded-md bg-primary px-3 py-1.5 text-xs text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-60"
              :disabled="!canSave"
            >
              {{ t("commands.save") }}
            </button>
          </footer>
        </form>
      </DialogContent>
    </DialogPortal>
  </DialogRoot>
</template>
