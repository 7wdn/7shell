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
import { useCommandsStore } from "@/stores/commands";
import type { Snippet } from "@/types/snippet";

const { t } = useI18n();
const commands = useCommandsStore();

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

    <p v-if="(shown?.commands.length ?? 0) === 0" class="text-xs text-muted-foreground">
      {{ t("commands.empty") }}
    </p>

    <ul class="space-y-2">
      <li
        v-for="snippet in shown?.commands ?? []"
        :key="snippet.id"
        data-command-row
        class="flex items-start gap-3 rounded-md border border-border bg-muted/20 px-3 py-2.5"
        :class="snippet.enabled ? '' : 'opacity-60'"
      >
        <div class="min-w-0 flex-1">
          <div class="flex items-center gap-2">
            <span class="truncate text-sm font-medium">{{ snippet.name }}</span>
            <kbd
              v-if="snippet.prefix !== ''"
              class="rounded-sm border border-border bg-background px-1.5 py-0.5 font-mono text-xs"
            >
              {{ snippet.prefix }}
            </kbd>
            <span v-else class="text-xs text-muted-foreground">{{ t("settings.commands.none") }}</span>
            <span v-if="!snippet.enabled" class="text-xs text-muted-foreground">
              {{ t("settings.commands.disabled") }}
            </span>
          </div>
          <p class="mt-1 truncate font-mono text-xs text-muted-foreground" :title="snippet.command">
            {{ snippet.command }}
          </p>
        </div>

        <Switch
          :model-value="snippet.enabled"
          :aria-label="`${t('settings.commands.enabled')}: ${snippet.name}`"
          @update:model-value="(value: boolean) => setEnabled(snippet, value)"
        />

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
      </li>
    </ul>

    <CommandDialog v-model:open="open" />
  </div>
</template>
