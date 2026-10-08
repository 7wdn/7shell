<script setup lang="ts">
/**
 * Writing a command: what it is called, what brings it up, and the line itself.
 *
 * A dialog rather than a form inside the page, for the reason the session editor is one: what is
 * being written is a thing of its own, the fields are not what the page around them is about, and
 * finishing or abandoning it should not rearrange the list somebody was reading.
 *
 * The draft lives in the commands store — the same one the panel under a terminal edits — so this
 * dialog and that panel are two ways into one draft rather than two drafts that have to be kept in
 * step.
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
import { computed } from "vue";
import { useI18n } from "vue-i18n";

import { useCommandsStore } from "@/stores/commands";

const props = defineProps<{ open: boolean }>();
const emit = defineEmits<{ "update:open": [value: boolean] }>();

const { t } = useI18n();
const commands = useCommandsStore();

/** A new command or one being changed: the two are one dialog, and the title says which. */
const editing = computed(() => (commands.draft?.id ?? "") !== "");

/**
 * The trigger is one word, because it is matched against one word.
 *
 * A trigger with a space in it is not a trigger that sometimes fails: the command line is split into
 * words before anything is matched, so the half before the space could never answer. Said here rather
 * than written down and met later as "the trigger does not work".
 */
const prefixInvalid = computed(() => /\s/.test(commands.draft?.prefix ?? ""));

/** What the store insists on before anything is written. */
const canSave = computed(() => {
  const draft = commands.draft;
  return draft !== null && draft.name.trim() !== "" && draft.command.trim() !== "" && !prefixInvalid.value;
});

/**
 * Closing the dialog ends the draft behind it.
 *
 * The draft belongs to the store, so leaving it standing would leave the panel under a terminal
 * editing a command nobody is looking at any more. Closing the dialog — the X, Escape, the overlay,
 * or the button — is one ending, and it means the same thing in all four.
 */
function updateOpen(value: boolean): void {
  if (!value) {
    commands.closeDraft();
  }
  emit("update:open", value);
}

async function save(): Promise<void> {
  await commands.save();
  if (commands.failure === null) {
    commands.closeDraft();
    emit("update:open", false);
  }
}
</script>

<template>
  <DialogRoot :open="props.open" @update:open="updateOpen">
    <DialogPortal>
      <DialogOverlay class="fixed inset-0 z-50 bg-black/50" />
      <DialogContent
        class="fixed top-1/2 left-1/2 z-50 w-[min(38rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border border-border bg-card shadow-xl focus:outline-none"
      >
        <header class="flex items-center justify-between border-b border-border px-4 py-3">
          <DialogTitle class="text-sm font-medium">
            {{ editing ? t("settings.commands.editTitle") : t("settings.commands.addTitle") }}
          </DialogTitle>
          <DialogDescription class="sr-only">{{ t("settings.commands.hint") }}</DialogDescription>
          <DialogClose
            class="grid size-7 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :aria-label="t('common.action.close')"
          >
            <X :size="15" />
          </DialogClose>
        </header>

        <form v-if="commands.draft" class="max-h-[70vh] space-y-4 overflow-auto px-4 py-4" @submit.prevent="save()">
          <div>
            <label class="block text-xs font-medium" for="command-name">{{ t("commands.name") }}</label>
            <input
              id="command-name"
              v-model="commands.draft.name"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 text-sm focus:border-primary focus:outline-none"
              :placeholder="t('settings.commands.namePlaceholder')"
            />
          </div>

          <div>
            <label class="block text-xs font-medium" for="command-trigger">{{ t("commands.trigger.label") }}</label>
            <input
              id="command-trigger"
              v-model="commands.draft.prefix"
              spellcheck="false"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus:border-primary focus:outline-none"
              :placeholder="t('commands.trigger.placeholder')"
            />
            <p v-if="prefixInvalid" class="mt-1 text-xs text-destructive">
              {{ t("settings.commands.prefixInvalid") }}
            </p>
            <p v-else class="mt-1 text-xs text-muted-foreground">{{ t("commands.trigger.hint") }}</p>
          </div>

          <div>
            <label class="block text-xs font-medium" for="command-line">{{ t("commands.command") }}</label>
            <textarea
              id="command-line"
              v-model="commands.draft.command"
              rows="6"
              spellcheck="false"
              class="mt-1 w-full rounded-md border border-border bg-background px-3 py-2 font-mono text-sm focus:border-primary focus:outline-none"
              :placeholder="t('settings.commands.linePlaceholder')"
            ></textarea>
          </div>

          <div class="flex flex-wrap items-center gap-4">
            <label class="flex items-center gap-2 text-xs">
              <input v-model="commands.draft.appendCr" type="checkbox" class="size-3.5 accent-primary" />
              {{ t("commands.appendCr") }}
            </label>
            <label class="flex items-center gap-2 text-xs">
              <input v-model="commands.draft.enabled" type="checkbox" class="size-3.5 accent-primary" />
              {{ t("settings.commands.enabled") }}
            </label>
          </div>

          <p
            v-if="commands.failure"
            class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
          >
            {{ t("settings.commands.saveFailed") }}
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
