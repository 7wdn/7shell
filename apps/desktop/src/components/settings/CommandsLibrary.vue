<script setup lang="ts">
/**
 * The command library, on the settings page.
 *
 * The same library the panel under a terminal edits — one store, two drawings of it: the panel is
 * where a command is written and sent, and this is where somebody goes looking through what they
 * have. A change made here shows up there without anything being copied, which is the point of
 * keeping the data in one place.
 *
 * Each command has a name, an optional trigger, and the line itself. The trigger is the short text
 * typed in the command line to bring the line up, which is what makes three letters worth more than
 * a whole name; a command without one is found by its name and by the line.
 */
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import { Pencil, Plus, Save, Trash2 } from "@lucide/vue";

import { useCommandsStore } from "@/stores/commands";

const { t } = useI18n();
const commands = useCommandsStore();

/** Which group is open, so a library of several does not arrive as one long list. */
const openGroup = ref<string>("");

onMounted(async () => {
  await commands.load();
  openGroup.value = commands.groups[0]?.id ?? "";
});

const shown = computed(() => commands.groups.find((group) => group.id === openGroup.value) ?? null);

/** Whether the form can be saved: a name and a line, which is what the store insists on too. */
const canSave = computed(() => {
  const draft = commands.draft;
  return draft !== null && draft.name.trim() !== "" && draft.command.trim() !== "";
});

/** Editing one command at a time, and the one being edited is the draft the store holds. */
function editing(id: string): boolean {
  return commands.draft?.id === id;
}

function startNew(): void {
  const group = shown.value ?? commands.groups[0];
  if (group) {
    commands.startNew(group.id);
  }
}
</script>

<template>
  <div class="space-y-5">
    <p class="text-xs text-muted-foreground">{{ t("settings.commands.hint") }}</p>

    <div class="flex items-center justify-between gap-2">
      <div class="flex flex-wrap items-center gap-1">
        <button
          v-for="group in commands.groups"
          :key="group.id"
          type="button"
          class="rounded-md px-2 py-0.5 text-[11px] transition-colors"
          :class="group.id === openGroup ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-accent/50'"
          @click="openGroup = group.id"
        >
          {{ group.name === "" ? t("commands.group.default") : group.name }}
        </button>
      </div>

      <button
        type="button"
        class="flex shrink-0 items-center gap-1.5 rounded-md border border-border px-2 py-1 text-[11px] transition-colors hover:bg-accent"
        @click="startNew"
      >
        <Plus :size="12" />
        {{ t("settings.commands.add") }}
      </button>
    </div>

    <!-- The line being written or changed, which is the same draft the panel under the terminal edits. -->
    <div v-if="commands.draft" class="space-y-2 rounded-md border border-primary/40 p-2">
      <div class="flex items-center gap-2">
        <input
          v-model="commands.draft.name"
          class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-xs focus:border-primary focus:outline-none"
          :placeholder="t('commands.name')"
          :aria-label="t('commands.name')"
        />
        <input
          v-model="commands.draft.prefix"
          spellcheck="false"
          class="w-28 shrink-0 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:border-primary focus:outline-none"
          :placeholder="t('commands.trigger.placeholder')"
          :aria-label="t('commands.trigger.label')"
        />
      </div>
      <textarea
        v-model="commands.draft.command"
        rows="3"
        spellcheck="false"
        class="w-full resize-y rounded-md border border-border bg-background px-2 py-1.5 font-mono text-xs focus:border-primary focus:outline-none"
        :placeholder="t('commands.command')"
        :aria-label="t('commands.command')"
      ></textarea>
      <p class="text-[11px] text-muted-foreground">{{ t("commands.trigger.hint") }}</p>

      <div class="flex items-center gap-2">
        <button
          type="button"
          class="flex items-center gap-1.5 rounded-md bg-primary px-2 py-1 text-[11px] text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
          :disabled="!canSave"
          @click="commands.save()"
        >
          <Save :size="12" />
          {{ t("commands.save") }}
        </button>
        <button
          type="button"
          class="rounded-md px-2 py-1 text-[11px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="commands.closeDraft()"
        >
          {{ t("common.action.close") }}
        </button>
      </div>
    </div>

    <!-- The library, as a table: what each command is called, what brings it up, and the line. -->
    <div class="overflow-hidden rounded-md border border-border">
      <div
        class="grid grid-cols-[1fr_7rem_1fr_4.5rem] gap-2 border-b border-border bg-muted/30 px-2 py-1.5 text-[11px] text-muted-foreground"
      >
        <span>{{ t("commands.name") }}</span>
        <span>{{ t("commands.trigger.label") }}</span>
        <span>{{ t("commands.command") }}</span>
        <span class="sr-only">{{ t("settings.commands.actions") }}</span>
      </div>

      <p v-if="(shown?.commands.length ?? 0) === 0" class="px-2 py-3 text-[11px] text-muted-foreground">
        {{ t("commands.empty") }}
      </p>

      <div
        v-for="snippet in shown?.commands ?? []"
        :key="snippet.id"
        data-command-row
        class="grid grid-cols-[1fr_7rem_1fr_4.5rem] items-center gap-2 border-b border-border px-2 py-1.5 text-xs last:border-b-0"
      >
        <span class="min-w-0 truncate" :title="snippet.name">{{ snippet.name }}</span>
        <span v-if="snippet.prefix !== ''" class="min-w-0">
          <kbd class="rounded-sm border border-border bg-muted px-1.5 py-0.5 font-mono text-[11px]">
            {{ snippet.prefix }}
          </kbd>
        </span>
        <span v-else class="min-w-0 text-[11px] text-muted-foreground">{{ t("settings.commands.none") }}</span>
        <span class="min-w-0 truncate font-mono text-[11px] text-muted-foreground" :title="snippet.command">
          {{ snippet.command }}
        </span>

        <span class="flex shrink-0 items-center justify-end gap-1">
          <button
            type="button"
            class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            :title="t('settings.commands.edit')"
            :aria-label="`${t('settings.commands.edit')}: ${snippet.name}`"
            :disabled="editing(snippet.id)"
            @click="commands.edit(snippet)"
          >
            <Pencil :size="11" />
          </button>
          <button
            type="button"
            class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-destructive/10 hover:text-destructive"
            :title="t('commands.remove')"
            :aria-label="`${t('commands.remove')}: ${snippet.name}`"
            @click="commands.remove(snippet.id)"
          >
            <Trash2 :size="11" />
          </button>
        </span>
      </div>
    </div>
  </div>
</template>
