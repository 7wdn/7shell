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
 * Both halves are listed, because this is also where somebody looks to see what is already offered.
 * The shipped rows are read-only and say so; a word added with the same command and word takes that
 * row's place rather than standing beside it, which is what makes changing one of ours possible
 * without a second kind of row.
 */
import { Pencil, Plus, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import WordDialog from "@/components/settings/WordDialog.vue";
import { SUBCOMMAND_TABLE, subcommandKeyFor } from "@/lib/commands/subcommands";
import { wordRows, type WordRow } from "@/lib/commands/words";
import { TABLE, rowClass } from "@/lib/table";
import { useWordsStore } from "@/stores/words";
import type { CompletionWord } from "@/types/snippet";

const { t, te } = useI18n();
const words = useWordsStore();

/** Whether the dialog is up, and which word it is about — `null` when it is a new one. */
const open = ref(false);
const editing = ref<CompletionWord | null>(null);

/** Their words first, then what this build ships: the same order the command line offers them in. */
const rows = computed(() => wordRows(words.words, SUBCOMMAND_TABLE));

onMounted(async () => {
  await words.load();
});

/** What a row says a word means: their own line, the one we ship, or what kind of word it is. */
function noteOf(row: WordRow): string {
  if (row.stored !== null && row.stored.note !== "") {
    return row.stored.note;
  }
  const key = subcommandKeyFor(row.command, row.word);
  return te(key) ? t(key) : t("commands.completion.kind.subcommand");
}

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
    <p class="text-xs text-muted-foreground">{{ t("settings.words.builtinHint") }}</p>

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

    <!-- The same table vocabulary the library uses: a heading, a row per reading. There is no empty
         row, because the table is never empty — what this build ships is in it from the start. -->
    <table class="table-fixed" :class="TABLE.page.table" :aria-label="t('settings.nav.words')">
      <thead :class="TABLE.page.head">
        <tr :class="TABLE.page.headRow">
          <th class="w-[22%] text-left" :class="TABLE.page.headCell">
            {{ t("settings.words.columns.command") }}
          </th>
          <th class="w-[18%] text-left" :class="TABLE.page.headCell">
            {{ t("settings.words.columns.word") }}
          </th>
          <th class="text-left" :class="TABLE.page.headCell">{{ t("settings.words.columns.note") }}</th>
          <th class="w-[14%] text-left" :class="TABLE.page.headCell">
            {{ t("settings.words.columns.source") }}
          </th>
          <th class="w-20" :class="TABLE.page.headCell">
            <span class="sr-only">{{ t("settings.words.columns.actions") }}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="row in rows"
          :key="row.key"
          data-word-row
          :class="[rowClass('page'), row.builtin ? 'text-muted-foreground' : '']"
        >
          <td class="truncate font-mono" :class="TABLE.page.cell" :title="row.command">{{ row.command }}</td>
          <td class="truncate font-mono" :class="TABLE.page.cell" :title="row.word">{{ row.word }}</td>
          <td class="truncate" :class="TABLE.page.cell" :title="noteOf(row)">{{ noteOf(row) }}</td>
          <td class="truncate" :class="TABLE.page.cell">
            {{ row.builtin ? t("settings.words.source.builtin") : t("settings.words.source.mine") }}
          </td>
          <td :class="TABLE.page.cell">
            <span v-if="row.stored" class="flex items-center justify-end gap-1">
              <button
                type="button"
                class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                :title="t('settings.commands.edit')"
                :aria-label="`${t('settings.commands.edit')}: ${row.command} ${row.word}`"
                @click="edit(row.stored)"
              >
                <Pencil :size="12" />
              </button>
              <button
                type="button"
                class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
                :title="t('commands.remove')"
                :aria-label="`${t('commands.remove')}: ${row.command} ${row.word}`"
                @click="words.remove(row.key)"
              >
                <Trash2 :size="12" />
              </button>
            </span>
          </td>
        </tr>
      </tbody>
    </table>

    <WordDialog v-model:open="open" :word="editing" />
  </div>
</template>
