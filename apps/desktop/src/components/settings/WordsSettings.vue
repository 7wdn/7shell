<script setup lang="ts">
/**
 * The words that come after a command, on the settings page.
 *
 * What this page is for: the command line already offers what each command is usually followed by,
 * and that table is the same for everybody. A machine has tools the table has never heard of — an
 * in-house CLI, a package manager, the verbs somebody uses in their own cluster — and this is where
 * those go. A word added here is offered before a shipped word of the same standing, which is the
 * rule the command library has too: what somebody made comes before what we ship.
 *
 * A word belongs to a pair — the command and the word — so the table repeats the command on every
 * row rather than grouping: the list is read by scanning it, and a heading that has to be looked up
 * two rows above is a heading nobody reads.
 */
import { Pencil, Plus, Trash2 } from "@lucide/vue";
import { onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import WordDialog from "@/components/settings/WordDialog.vue";
import { TABLE, rowClass } from "@/lib/table";
import { useWordsStore } from "@/stores/words";
import type { CompletionWord } from "@/types/snippet";

const { t } = useI18n();
const words = useWordsStore();

/** Whether the dialog is up, and which word it is about — `null` when it is a new one. */
const open = ref(false);
const editing = ref<CompletionWord | null>(null);

onMounted(async () => {
  await words.load();
});

function add(): void {
  editing.value = null;
  open.value = true;
}

function edit(word: CompletionWord): void {
  editing.value = word;
  open.value = true;
}
</script>

<template>
  <div class="space-y-5">
    <p class="text-xs text-muted-foreground">{{ t("settings.words.hint") }}</p>

    <p v-if="words.failure" class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs">
      {{ t("settings.words.saveFailed") }}
    </p>

    <div class="flex flex-wrap items-center justify-end gap-3">
      <button
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs transition-colors hover:bg-accent"
        @click="add"
      >
        <Plus :size="13" />
        {{ t("settings.words.add") }}
      </button>
    </div>

    <!-- The same table vocabulary the library uses: a heading, a row per reading, and an empty list
         drawn as a row of the table rather than as a sentence under it. -->
    <table class="table-fixed" :class="TABLE.page.table" :aria-label="t('settings.nav.words')">
      <thead :class="TABLE.page.head">
        <tr :class="TABLE.page.headRow">
          <th class="w-[24%] text-left" :class="TABLE.page.headCell">
            {{ t("settings.words.columns.command") }}
          </th>
          <th class="w-[20%] text-left" :class="TABLE.page.headCell">
            {{ t("settings.words.columns.word") }}
          </th>
          <th class="text-left" :class="TABLE.page.headCell">{{ t("settings.words.columns.note") }}</th>
          <th class="w-20" :class="TABLE.page.headCell">
            <span class="sr-only">{{ t("settings.words.columns.actions") }}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        <tr v-for="entry in words.words" :key="entry.id" data-word-row :class="rowClass('page')">
          <td class="truncate font-mono" :class="TABLE.page.cell" :title="entry.command">{{ entry.command }}</td>
          <td class="truncate font-mono" :class="TABLE.page.cell" :title="entry.word">{{ entry.word }}</td>
          <td class="truncate" :class="TABLE.page.cell" :title="entry.note">{{ entry.note }}</td>
          <td :class="TABLE.page.cell">
            <span class="flex items-center justify-end gap-1">
              <button
                type="button"
                class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                :title="t('settings.commands.edit')"
                :aria-label="`${t('settings.commands.edit')}: ${entry.command} ${entry.word}`"
                @click="edit(entry)"
              >
                <Pencil :size="12" />
              </button>
              <button
                type="button"
                class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
                :title="t('commands.remove')"
                :aria-label="`${t('commands.remove')}: ${entry.command} ${entry.word}`"
                @click="words.remove(entry.id)"
              >
                <Trash2 :size="12" />
              </button>
            </span>
          </td>
        </tr>

        <tr v-if="words.words.length === 0">
          <td colspan="4" :class="TABLE.page.emptyCell">{{ t("settings.words.empty") }}</td>
        </tr>
      </tbody>
    </table>

    <WordDialog v-model:open="open" :word="editing" />
  </div>
</template>
