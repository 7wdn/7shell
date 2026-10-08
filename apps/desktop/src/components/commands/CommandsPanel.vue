<script setup lang="ts">
/**
 * The commands panel: what somebody runs often, and the button that types it into a shell.
 *
 * It lives under the terminal rather than in a page of its own, because that is where the command
 * is going: the left half is the library (groups, then the commands in the chosen one), the right
 * half is the line being edited, and the bottom bar is where it goes — this session, or every
 * session that is open. A saved line with `#1`…`#5` in it gets a box per slot, filled in before the
 * send button is reachable, so what is sent is a line somebody can read.
 *
 * The verdict and the send are one flow: the first press may come back with reasons instead of a
 * send, and then the button says so and asks again. That is the only case where anything waits, and
 * it only happens when more than one shell is being written to.
 */
import { ChevronDown, ChevronUp, FolderPlus, Pencil, Play, Plus, Save, Trash2, TriangleAlert } from "@lucide/vue";
import { computed, onMounted, ref } from "vue";
import { useI18n } from "vue-i18n";

import Select, { type SelectOption } from "@/components/ui/Select.vue";
import ConfirmDialog from "@/components/ui/ConfirmDialog.vue";
import { describeErrorFields, describeShellError } from "@/i18n/errors";
import { atLeast, needsApproval } from "@/lib/batch/risk";
import { DEFAULT_GROUP_ID } from "@/lib/commands/defaults";
import { useCommandsStore } from "@/stores/commands";
import { useTerminalsStore } from "@/stores/terminals";

const { t } = useI18n();
const commands = useCommandsStore();
const terminals = useTerminalsStore();

/** Which group the list on the left is showing. */
const openGroup = ref<string>("");

const newGroupName = ref("");
const addingGroup = ref(false);

/**
 * The group somebody asked to remove.
 *
 * A question rather than an immediate delete: taking a group takes the commands in it, and a
 * command is a line somebody wrote — the one thing in this panel that cannot be typed again from
 * memory.
 */
const removing = ref<{ id: string; name: string; count: number } | null>(null);

/** The group being renamed, and what the field holds while it is. */
const renaming = ref<{ id: string; name: string } | null>(null);

onMounted(async () => {
  await commands.load();
  openGroup.value = commands.groups[0]?.id ?? "";
});

const shown = computed(() => commands.groups.find((group) => group.id === openGroup.value) ?? null);
const history = computed(() => commands.history.slice(0, 12));

const targetOptions = computed<SelectOption[]>(() => [
  { value: "session", label: t("commands.target.session") },
  { value: "all", label: t("commands.target.all", { count: terminals.writableHandles().length }) },
]);

function translate(key: string, params?: Record<string, string>): string {
  return params ? t(key, params) : t(key);
}

const problem = computed(() => (commands.failure ? describeShellError(translate, commands.failure) : null));

/**
 * The terminals the last send could not reach.
 *
 * Each with its own reason, because "it reached three of them" is the answer somebody acts on when
 * the fourth tab has ended.
 */
const unreached = computed(() =>
  commands.sendFailures.map((failure) => ({ failure, described: describeErrorFields(translate, failure) })),
);

/** Whether the send button can do anything: a line, a target with a shell behind it, and no wait. */
const canSend = computed(() => {
  if (commands.sending || commands.filled.trim() === "") {
    return false;
  }
  if (commands.approval && !commands.acknowledged) {
    return false;
  }
  if (commands.target === "all") {
    return terminals.writableHandles().length > 0;
  }
  return terminals.activeTerminalHandle() !== null;
});

/** The last value used in a slot, offered back: the reason slots are remembered at all. */
function suggestionsFor(slot: number): string[] {
  const stored = commands.groups.flatMap((group) => group.commands).find((entry) => entry.id === commands.draft?.id);
  return [...(stored?.slotHistory[slot - 1] ?? [])];
}

/**
 * Whether the command being edited can move further in that direction.
 *
 * Asked of the list rather than kept as state: the first command cannot go up and the last cannot
 * go down, and a button that looked pressable and did nothing would be worse than one that is dim.
 */
function canMove(direction: -1 | 1): boolean {
  const draft = commands.draft;
  if (!draft?.id) {
    return false;
  }

  const group = commands.groups.find((entry) => entry.commands.some((command) => command.id === draft.id));
  if (!group) {
    return false;
  }

  const index = group.commands.findIndex((command) => command.id === draft.id);
  return index >= 0 && group.commands[index + direction] !== undefined;
}

async function saveGroup(): Promise<void> {
  if (newGroupName.value.trim() === "") {
    return;
  }
  await commands.addGroup(newGroupName.value);
  newGroupName.value = "";
  addingGroup.value = false;
}

/** Finishes a rename, whichever way it was started. */
async function renameGroup(): Promise<void> {
  const target = renaming.value;
  if (!target) {
    return;
  }
  await commands.renameGroup(target.id, newGroupName.value);
  renaming.value = null;
  newGroupName.value = "";
}
</script>

<template>
  <div class="flex h-full min-h-0 text-xs">
    <!-- The library. Groups first, then the commands in the one that is open. -->
    <aside class="flex w-64 shrink-0 flex-col border-r border-border">
      <header class="flex items-center justify-between border-b border-border px-2 py-1.5">
        <span class="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("commands.groups") }}
        </span>
        <button
          type="button"
          class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          :title="t('commands.group.new')"
          :aria-label="t('commands.group.new')"
          @click="addingGroup = !addingGroup"
        >
          <FolderPlus :size="12" />
        </button>
      </header>

      <div v-if="addingGroup || renaming" class="flex items-center gap-1 border-b border-border px-2 py-1.5">
        <input
          v-model="newGroupName"
          class="min-w-0 flex-1 rounded-sm border border-border bg-background px-1.5 py-1 text-[11px] focus:outline-none"
          :placeholder="t('commands.group.name')"
          @keydown.enter="renaming ? renameGroup() : saveGroup()"
        />
        <button
          type="button"
          class="rounded-sm border border-border px-1.5 py-1 text-[11px]"
          @click="renaming ? renameGroup() : saveGroup()"
        >
          {{ renaming ? t("commands.group.rename") : t("commands.group.add") }}
        </button>
        <button
          type="button"
          class="rounded-sm px-1 py-1 text-[11px] text-muted-foreground hover:text-foreground"
          @click="
            renaming = null;
            addingGroup = false;
            newGroupName = '';
          "
        >
          {{ t("common.action.close") }}
        </button>
      </div>

      <ul class="shrink-0 overflow-auto border-b border-border py-1">
        <li v-for="group in commands.groups" :key="group.id">
          <div
            class="group flex items-center gap-1 px-2 py-1 transition-colors"
            :class="group.id === openGroup ? 'bg-accent/60' : 'hover:bg-accent/30'"
          >
            <button type="button" class="min-w-0 flex-1 truncate text-left" @click="openGroup = group.id">
              {{ group.name === "" ? t("commands.group.default") : group.name }}
            </button>
            <span class="shrink-0 text-[10px] text-muted-foreground">{{ group.commands.length }}</span>
            <button
              v-if="group.id !== DEFAULT_GROUP_ID"
              type="button"
              class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 hover:bg-accent hover:text-foreground"
              :title="t('commands.group.rename')"
              :aria-label="t('commands.group.rename')"
              @click="
                renaming = { id: group.id, name: group.name };
                newGroupName = group.name;
              "
            >
              <Pencil :size="10" />
            </button>
            <button
              v-if="group.id !== DEFAULT_GROUP_ID"
              type="button"
              class="grid size-4 shrink-0 place-items-center rounded-sm text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 hover:bg-accent hover:text-foreground"
              :title="t('commands.group.remove')"
              :aria-label="t('commands.group.remove')"
              @click="removing = { id: group.id, name: group.name, count: group.commands.length }"
            >
              <Trash2 :size="10" />
            </button>
          </div>
        </li>
      </ul>

      <header class="flex items-center justify-between border-b border-border px-2 py-1.5">
        <span class="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("commands.saved") }}
        </span>
        <button
          type="button"
          class="grid size-5 place-items-center rounded-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-40"
          :disabled="shown === null"
          :title="t('commands.new')"
          :aria-label="t('commands.new')"
          @click="shown && commands.startNew(shown.id)"
        >
          <Plus :size="12" />
        </button>
      </header>

      <ul class="min-h-0 flex-1 overflow-auto py-1">
        <li v-for="snippet in shown?.commands ?? []" :key="snippet.id">
          <button
            type="button"
            class="block w-full truncate px-2 py-1 text-left font-mono transition-colors"
            :class="snippet.id === commands.draft?.id ? 'bg-accent/60' : 'hover:bg-accent/30'"
            :title="snippet.command"
            @click="commands.edit(snippet)"
          >
            {{ snippet.name }}
          </button>
        </li>
        <li v-if="(shown?.commands.length ?? 0) === 0" class="px-2 py-2 text-[11px] text-muted-foreground">
          {{ t("commands.empty") }}
        </li>
      </ul>

      <!--
        What was sent, which is the other half of "keep the lines you run often": the one worth
        keeping is usually the one that was just typed.
      -->
      <header class="flex items-center justify-between border-t border-border px-2 py-1.5">
        <span class="text-[11px] font-medium tracking-wide text-muted-foreground uppercase">
          {{ t("commands.history") }}
        </span>
        <button
          type="button"
          class="rounded-sm px-1 text-[10px] text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          @click="commands.clearHistory()"
        >
          {{ t("commands.historyClear") }}
        </button>
      </header>
      <ul class="max-h-32 shrink-0 overflow-auto py-1">
        <li v-for="(entry, index) in history" :key="`${entry.at}-${index}`">
          <button
            type="button"
            class="block w-full truncate px-2 py-0.5 text-left font-mono text-[10px] text-muted-foreground transition-colors hover:bg-accent/40 hover:text-foreground"
            :title="entry.command"
            @click="commands.editFromHistory(entry)"
          >
            {{ entry.command }}
          </button>
        </li>
      </ul>
    </aside>

    <!-- The line being edited, its slots, and where it goes. -->
    <section class="flex min-w-0 flex-1 flex-col">
      <div v-if="commands.draft" class="flex min-h-0 flex-1 flex-col gap-2 p-3">
        <div class="flex items-center gap-2">
          <input
            v-model="commands.draft.name"
            class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 text-xs focus:border-primary focus:outline-none"
            :placeholder="t('commands.name')"
          />
          <!--
            The trigger: the short text that brings this line up in the command line. Optional, and
            shorter than the name on purpose — it is what somebody types, not what they read.
          -->
          <input
            v-model="commands.draft.prefix"
            spellcheck="false"
            class="w-28 shrink-0 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:border-primary focus:outline-none"
            :placeholder="t('commands.trigger.placeholder')"
            :aria-label="t('commands.trigger.label')"
          />
          <label class="flex shrink-0 items-center gap-1.5 text-[11px] text-muted-foreground">
            <input v-model="commands.draft.appendCr" type="checkbox" class="size-3.5 accent-primary" />
            {{ t("commands.appendCr") }}
          </label>
          <label class="flex shrink-0 items-center gap-1.5 text-[11px] text-muted-foreground">
            <input v-model="commands.draft.enabled" type="checkbox" class="size-3.5 accent-primary" />
            {{ t("settings.commands.enabled") }}
          </label>
        </div>

        <textarea
          v-model="commands.draft.command"
          rows="4"
          spellcheck="false"
          :aria-label="t('commands.command')"
          class="w-full resize-y rounded-md border border-border bg-background px-2 py-1.5 font-mono text-xs focus:border-primary focus:outline-none"
          :placeholder="t('commands.command')"
        ></textarea>

        <div v-if="commands.slots.length > 0" class="grid gap-2 sm:grid-cols-2">
          <label v-for="slot in commands.slots" :key="slot" class="flex items-center gap-2">
            <span class="shrink-0 font-mono text-[11px] text-muted-foreground">#{{ slot }}</span>
            <input
              :value="commands.values[slot - 1]"
              :list="`slot-history-${slot}`"
              class="min-w-0 flex-1 rounded-md border border-border bg-background px-2 py-1 font-mono text-xs focus:border-primary focus:outline-none"
              @input="commands.setValue(slot - 1, ($event.target as HTMLInputElement).value)"
            />
            <!--
              One list per slot. A single shared list answered every box with what the first box had
              used before, which for a command like `cp #1 #2` is an answer to the wrong question.
            -->
            <datalist :id="`slot-history-${slot}`">
              <option v-for="value in suggestionsFor(slot)" :key="value" :value="value"></option>
            </datalist>
          </label>
        </div>

        <p v-if="commands.slots.length > 0" class="font-mono text-[11px] break-all text-muted-foreground">
          {{ commands.filled }}
        </p>

        <div class="flex-1"></div>

        <div v-if="problem" class="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5">
          <p>{{ problem.message }}</p>
          <p class="font-mono text-[10px] text-muted-foreground">{{ problem.code }}</p>
        </div>

        <ul v-if="unreached.length > 0" class="rounded-md border border-destructive/40 bg-destructive/10 px-2 py-1.5">
          <li v-for="entry in unreached" :key="entry.failure.handle" class="flex items-baseline gap-2">
            <span class="shrink-0 font-mono text-[10px] text-muted-foreground">{{ entry.failure.handle }}</span>
            <span class="min-w-0 flex-1 truncate" :title="entry.described.detail ?? entry.described.code">
              {{ entry.described.message }}
            </span>
          </li>
        </ul>

        <!-- The verdict, when the send was stopped by it: what it says and the button that agrees. -->
        <div
          v-if="commands.approval"
          class="rounded-md border px-2 py-1.5"
          :class="
            atLeast(commands.approval.level, 'destructive')
              ? 'border-destructive/40 bg-destructive/10'
              : 'border-border bg-background'
          "
        >
          <div class="flex items-center gap-2">
            <TriangleAlert :size="12" />
            <span class="font-medium">{{ t(`risk.level.${commands.approval.level}`) }}</span>
            <span class="text-[11px] opacity-80">
              {{ commands.approval.reasons.map((reason) => t(reason.labelKey)).join(" · ") }}
            </span>
          </div>
          <label v-if="needsApproval(commands.approval.level)" class="mt-1 flex items-center gap-2 text-[11px]">
            <input
              type="checkbox"
              class="size-3.5 accent-primary"
              :checked="commands.acknowledged"
              @change="commands.acknowledged = true"
            />
            {{ t("commands.confirm", { count: terminals.writableHandles().length }) }}
          </label>
        </div>

        <div class="flex flex-wrap items-center gap-2">
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-border px-2 py-1 transition-colors hover:bg-accent disabled:opacity-40"
            :disabled="commands.draft.name.trim() === '' || commands.draft.command.trim() === ''"
            @click="commands.save()"
          >
            <Save :size="12" />
            {{ t("commands.save") }}
          </button>
          <button
            v-if="commands.draft.id"
            type="button"
            class="grid size-6 place-items-center rounded-md border border-border transition-colors hover:bg-accent disabled:opacity-40"
            :disabled="!canMove(-1)"
            :title="t('commands.moveUp')"
            :aria-label="t('commands.moveUp')"
            @click="commands.move(commands.draft.id, -1)"
          >
            <ChevronUp :size="12" />
          </button>
          <button
            v-if="commands.draft.id"
            type="button"
            class="grid size-6 place-items-center rounded-md border border-border transition-colors hover:bg-accent disabled:opacity-40"
            :disabled="!canMove(1)"
            :title="t('commands.moveDown')"
            :aria-label="t('commands.moveDown')"
            @click="commands.move(commands.draft.id, 1)"
          >
            <ChevronDown :size="12" />
          </button>
          <button
            v-if="commands.draft.id"
            type="button"
            class="flex items-center gap-1.5 rounded-md border border-destructive/40 px-2 py-1 text-destructive transition-colors hover:bg-destructive/10"
            @click="commands.remove(commands.draft.id)"
          >
            <Trash2 :size="12" />
            {{ t("commands.remove") }}
          </button>
          <button
            type="button"
            class="rounded-md px-2 py-1 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            @click="commands.closeDraft()"
          >
            {{ t("common.action.close") }}
          </button>

          <div class="flex-1"></div>

          <Select
            :model-value="commands.target"
            :options="targetOptions"
            size="compact"
            :aria-label="t('commands.target.label')"
            @update:model-value="commands.target = $event === 'all' ? 'all' : 'session'"
          />
          <button
            type="button"
            class="flex items-center gap-1.5 rounded-md bg-primary px-3 py-1 text-primary-foreground transition-colors hover:bg-primary/90 disabled:cursor-not-allowed disabled:opacity-50"
            :disabled="!canSend"
            @click="commands.send(commands.approval !== null && commands.acknowledged)"
          >
            <Play :size="12" />
            {{ commands.approval ? t("commands.sendAnyway") : t("commands.send") }}
          </button>
        </div>
      </div>

      <div v-else class="grid h-full place-items-center px-6 text-center">
        <p class="max-w-sm text-[11px] text-muted-foreground">{{ t("commands.hint") }}</p>
      </div>
    </section>

    <ConfirmDialog
      v-if="removing"
      :open="true"
      :title="t('commands.group.removeTitle')"
      :message="t('commands.group.removeMessage', { count: removing.count })"
      :confirm-label="t('commands.group.removeConfirm')"
      danger
      @confirm="
        commands.removeGroup(removing.id);
        removing = null;
      "
      @update:open="removing = null"
    />
  </div>
</template>
