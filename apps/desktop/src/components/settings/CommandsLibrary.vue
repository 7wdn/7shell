<script setup lang="ts">
/**
 * The command library, on the settings page.
 *
 * The same library the panel under a terminal edits — one store, two drawings of it: the panel is
 * where a command is written and sent, and this is where somebody goes looking through what they
 * have. A change made here shows up there without anything being copied, which is the point of
 * keeping the data in one place.
 *
 * A command is written in a dialog rather than in the list: what is being written is a thing of its
 * own, and the fields are not what the page around them is about. Everything on a row — the switch
 * included — is applied at once, so the list is the library as it stands rather than a form waiting
 * to be submitted.
 */
import { Pencil, Plus, Trash2 } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import CommandDialog from "@/components/settings/CommandDialog.vue";
import Switch from "@/components/ui/Switch.vue";
import { TABLE, rowClass } from "@/lib/table";
import { useCommandsStore } from "@/stores/commands";
import { usePreferencesStore } from "@/stores/preferences";
import type { Snippet } from "@/types/snippet";

const props = withDefaults(defineProps<{ control?: boolean }>(), { control: true });

const { t } = useI18n();
const commands = useCommandsStore();
const preferences = usePreferencesStore();

/** Which group the list is showing. */
const openGroup = ref<string>("");
/** Whether the dialog is up, and the draft behind it is the store's. */
const open = ref(false);
/** Why the last write to the library did not land, in the page's own words. */
const problem = ref(false);

onMounted(async () => {
  await commands.load();
  openGroup.value = commands.groups[0]?.id ?? "";
});

const shown = computed(() => commands.groups.find((group) => group.id === openGroup.value) ?? null);

function add(): void {
  const group = shown.value ?? commands.groups[0];
  if (group) {
    commands.startNew(group.id);
    problem.value = false;
    open.value = true;
  }
}

function edit(snippet: Snippet): void {
  commands.edit(snippet);
  problem.value = false;
  open.value = true;
}

/** The switch is the library's own, not a form's: switching it writes it down straight away. */
async function setEnabled(snippet: Snippet, enabled: boolean): Promise<void> {
  await commands.setEnabled(snippet, enabled);
  problem.value = commands.failure !== null;
}

async function remove(snippet: Snippet): Promise<void> {
  await commands.remove(snippet.id);
  problem.value = commands.failure !== null;
}
</script>

<template>
  <div class="space-y-5">
    <p class="text-xs text-muted-foreground">{{ t("settings.commands.hint") }}</p>

    <!--
      The switch that governs the whole feature, on the page that is about it. It sat under the
      terminal's settings until now, where it described something the terminal page does not draw: a
      menu beside the command line rather than anything about the terminal itself.
    -->
    <div
      v-if="props.control"
      class="flex items-center justify-between gap-3 rounded-md border border-border bg-muted/20 px-3 py-2.5"
    >
      <div class="min-w-0">
        <p class="text-sm font-medium">{{ t("settings.rows.commandCompletion.title") }}</p>
        <p class="mt-0.5 text-xs text-muted-foreground">{{ t("settings.rows.commandCompletion.summary") }}</p>
      </div>
      <Switch
        :model-value="preferences.commandCompletion"
        :aria-label="t('settings.rows.commandCompletion.title')"
        @update:model-value="(value: boolean) => preferences.setCommandCompletion(value)"
      />
    </div>

    <p
      v-if="problem || commands.failure"
      class="rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-xs"
    >
      {{ t("settings.commands.saveFailed") }}
    </p>

    <div class="flex flex-wrap items-center justify-between gap-3">
      <div class="flex flex-wrap items-center gap-1">
        <button
          v-for="group in commands.groups"
          :key="group.id"
          type="button"
          class="rounded-md px-2 py-1 text-xs transition-colors"
          :class="group.id === openGroup ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent/50'"
          @click="openGroup = group.id"
        >
          {{ group.name === "" ? t("commands.group.default") : group.name }}
        </button>
      </div>

      <button
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2.5 py-1.5 text-xs transition-colors hover:bg-accent"
        @click="add"
      >
        <Plus :size="13" />
        {{ t("settings.commands.add") }}
      </button>
    </div>

    <!--
      A table, because the library is a list of records: four facts about every command, read down
      their own columns — a name, the trigger that reaches it, whether it is on, and the line itself.
      The fifth column is the pair of things somebody does to a row. Drawn from the table vocabulary
      like every other table here, so a heading and a row read the same on this page as on the rest.
    -->
    <table class="table-fixed" :class="TABLE.page.table" :aria-label="t('settings.nav.commands')">
      <thead :class="TABLE.page.head">
        <tr :class="TABLE.page.headRow">
          <th class="w-[26%] text-left" :class="TABLE.page.headCell">
            {{ t("settings.commands.columns.name") }}
          </th>
          <th class="w-[12%] text-left" :class="TABLE.page.headCell">
            {{ t("settings.commands.columns.trigger") }}
          </th>
          <th class="w-[20%] text-left" :class="TABLE.page.headCell">
            {{ t("settings.commands.columns.status") }}
          </th>
          <th class="text-left" :class="TABLE.page.headCell">{{ t("settings.commands.columns.line") }}</th>
          <th class="w-20" :class="TABLE.page.headCell">
            <span class="sr-only">{{ t("settings.commands.columns.actions") }}</span>
          </th>
        </tr>
      </thead>
      <tbody>
        <tr
          v-for="snippet in shown?.commands ?? []"
          :key="snippet.id"
          data-command-row
          :class="[rowClass('page'), snippet.enabled ? '' : 'text-muted-foreground']"
        >
          <td class="truncate font-medium" :class="TABLE.page.cell" :title="snippet.name">{{ snippet.name }}</td>
          <td :class="TABLE.page.cell">
            <kbd v-if="snippet.prefix !== ''" class="rounded-sm border border-border bg-background px-1.5 font-mono">
              {{ snippet.prefix }}
            </kbd>
            <span v-else>{{ t("settings.commands.none") }}</span>
          </td>
          <td :class="TABLE.page.cell">
            <span class="flex items-center gap-2">
              <Switch
                :model-value="snippet.enabled"
                :aria-label="`${t('settings.commands.enabled')}: ${snippet.name}`"
                @update:model-value="(value: boolean) => setEnabled(snippet, value)"
              />
              <span class="truncate">
                {{ snippet.enabled ? t("settings.commands.enabled") : t("settings.commands.disabled") }}
              </span>
            </span>
          </td>
          <td :class="TABLE.page.cell">
            <span class="block truncate font-mono" :title="snippet.command">{{ snippet.command }}</span>
          </td>
          <td :class="TABLE.page.cell">
            <span class="flex items-center justify-end gap-1">
              <button
                type="button"
                class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                :title="t('settings.commands.edit')"
                :aria-label="`${t('settings.commands.edit')}: ${snippet.name}`"
                @click="edit(snippet)"
              >
                <Pencil :size="12" />
              </button>
              <button
                type="button"
                class="grid size-6 shrink-0 place-items-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
                :title="t('commands.remove')"
                :aria-label="`${t('commands.remove')}: ${snippet.name}`"
                @click="remove(snippet)"
              >
                <Trash2 :size="12" />
              </button>
            </span>
          </td>
        </tr>

        <tr v-if="(shown?.commands.length ?? 0) === 0">
          <td colspan="5" :class="TABLE.page.emptyCell">{{ t("commands.empty") }}</td>
        </tr>
      </tbody>
    </table>

    <CommandDialog v-model:open="open" />
  </div>
</template>
