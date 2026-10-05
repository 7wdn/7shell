/**
 * The command library, and sending one of its lines into the shells that are open.
 *
 * Two things are decided here rather than in the panel, because they are about the data rather than
 * about the drawing:
 *
 * * **What a slot is.** `#1`…`#5` in a saved line are places values go, and the values used before
 *   are remembered per command per slot. A line with no slots sends as it stands; a line with slots
 *   sends with the values somebody typed into the boxes under it.
 * * **What the two send targets mean.** "This session" is one handle; "all sessions" is every
 *   terminal open right now. Both write into a shell that is already running — they do not open a
 *   connection and there is no exit code to read back.
 */

import { computed, ref } from "vue";
import { defineStore } from "pinia";

import { backend } from "@/lib/backend/api";
import { asShellError, type ShellError } from "@/lib/backend/errors";
import { useTerminalsStore } from "@/stores/terminals";
import type { RiskAssessment } from "@/types/batch";
import type { CommandHistoryEntry, Snippet, SnippetGroup } from "@/types/snippet";

/** How many `#n` slots a line may use. The same five the storage knows about. */
export const SLOTS = 5;

/** The slots a line uses, in order: `#1` in `service #1 restart` means `[1]`. */
export function slotsIn(command: string): number[] {
  const found = new Set<number>();
  for (const match of command.matchAll(/#([1-9])/g)) {
    const slot = Number(match[1]);
    if (slot <= SLOTS) {
      found.add(slot);
    }
  }
  return [...found].sort((left, right) => left - right);
}

/**
 * The line with its slots filled in.
 *
 * Substituted raw, the way the value would have been typed: these are shell lines, and a value
 * somebody wants quoted is one they quote. Slots left empty stay as they are, so a half-filled
 * command is visibly not a finished one rather than quietly something else.
 */
export function fillSlots(command: string, values: readonly string[]): string {
  return command.replace(/#([1-9])/g, (whole, digit: string) => {
    const value = values[Number(digit) - 1];
    return value === undefined || value.trim() === "" ? whole : value;
  });
}

/** A draft being edited: the fields the form owns, before it is a saved command. */
export interface SnippetDraft {
  id: string;
  groupId: string;
  name: string;
  command: string;
  appendCr: boolean;
}

export const useCommandsStore = defineStore("commands", () => {
  const groups = ref<SnippetGroup[]>([]);
  const history = ref<CommandHistoryEntry[]>([]);
  const loaded = ref(false);
  const failure = ref<ShellError | null>(null);

  /** The command being edited, and the values in its slot boxes. */
  const draft = ref<SnippetDraft | null>(null);
  const values = ref<string[]>(Array.from({ length: SLOTS }, () => ""));

  /** Whether this session or every session is the target. */
  const target = ref<"session" | "all">("session");
  /** The verdict for a send that asked first, and whether somebody has agreed. */
  const approval = ref<RiskAssessment | null>(null);
  const acknowledged = ref(false);
  const sending = ref(false);
  /** What the last send answered, per terminal, when something did not reach. */
  const sendFailure = ref<ShellError | null>(null);
  const lastSent = ref(0);

  const slots = computed(() => (draft.value ? slotsIn(draft.value.command) : []));
  const filled = computed(() => (draft.value ? fillSlots(draft.value.command, values.value) : ""));

  async function load(): Promise<void> {
    try {
      groups.value = await backend.commands.library();
      history.value = await backend.commands.history();
      failure.value = null;
      loaded.value = true;
    } catch (error) {
      failure.value = asShellError(error, "snippets_open");
    }
  }

  function edit(snippet: Snippet): void {
    draft.value = {
      id: snippet.id,
      groupId: snippet.groupId,
      name: snippet.name,
      command: snippet.command,
      appendCr: snippet.appendCr,
    };
    approval.value = null;
    acknowledged.value = false;
    // The last value used for each slot, which is what somebody changing one thing expects to find.
    values.value = Array.from({ length: SLOTS }, (_, slot) => snippet.slotHistory[slot]?.[0] ?? "");
  }

  function startNew(groupId: string): void {
    draft.value = { id: "", groupId, name: "", command: "", appendCr: true };
    values.value = Array.from({ length: SLOTS }, () => "");
    approval.value = null;
    acknowledged.value = false;
  }

  function closeDraft(): void {
    draft.value = null;
    approval.value = null;
    acknowledged.value = false;
  }

  function setValue(slot: number, value: string): void {
    const next = [...values.value];
    next[slot] = value;
    values.value = next;
    // Any edit after agreeing is a different command line, and the agreement was about the old one.
    approval.value = null;
    acknowledged.value = false;
  }

  /** Saves the draft, and keeps the values that were used with it. */
  async function save(): Promise<void> {
    const current = draft.value;
    if (!current || current.name.trim() === "" || current.command.trim() === "") {
      return;
    }

    try {
      const saved = await backend.commands.save({
        id: current.id,
        groupId: current.groupId,
        name: current.name.trim(),
        command: current.command,
        appendCr: current.appendCr,
        order: 0,
        slotHistory: current.id
          ? (groups.value.flatMap((group) => group.commands).find((entry) => entry.id === current.id)?.slotHistory ??
            [])
          : [],
      });
      draft.value = { ...current, id: saved.id };
      await load();
      failure.value = null;
    } catch (error) {
      failure.value = asShellError(error, "snippet_save");
    }
  }

  async function remove(id: string): Promise<void> {
    try {
      await backend.commands.delete(id);
      if (draft.value?.id === id) {
        closeDraft();
      }
      await load();
    } catch (error) {
      failure.value = asShellError(error, "snippet_delete");
    }
  }

  async function addGroup(name: string): Promise<void> {
    try {
      await backend.commands.saveGroup({ id: "", name: name.trim(), order: 0, commands: [] });
      await load();
    } catch (error) {
      failure.value = asShellError(error, "snippet_group_save");
    }
  }

  async function removeGroup(id: string): Promise<void> {
    try {
      await backend.commands.deleteGroup(id);
      if (draft.value?.groupId === id) {
        closeDraft();
      }
      await load();
    } catch (error) {
      failure.value = asShellError(error, "snippet_group_delete");
    }
  }

  async function clearHistory(): Promise<void> {
    try {
      await backend.commands.clearHistory();
      await load();
    } catch (error) {
      failure.value = asShellError(error, "command_history_clear");
    }
  }

  /** Puts a line from the history into the editor, without slots of its own to fill. */
  function editFromHistory(entry: CommandHistoryEntry): void {
    draft.value = {
      id: "",
      groupId: groups.value[0]?.id ?? "default",
      name: entry.command.slice(0, 40),
      command: entry.command,
      appendCr: true,
    };
    values.value = Array.from({ length: SLOTS }, () => "");
  }

  /**
   * Sends the filled line to the chosen shells.
   *
   * `agree` is for the second press: the first one may come back with a verdict instead, and the
   * button that follows says so. Only a send to more than one shell can be stopped that way, and
   * nothing has been written when it happens.
   */
  async function send(agree = false): Promise<void> {
    const terminals = useTerminalsStore();
    const line = filled.value;
    if (line.trim() === "" || sending.value) {
      return;
    }

    const handles =
      target.value === "all"
        ? terminals.writableHandles()
        : [terminals.activeTerminalHandle()].filter((handle): handle is string => handle !== null);
    if (handles.length === 0) {
      sendFailure.value = null;
      failure.value = null;
      return;
    }

    sending.value = true;
    sendFailure.value = null;
    try {
      const outcome = await backend.commands.send(
        { kind: "terminals", handles },
        line,
        draft.value?.appendCr ?? true,
        agree,
      );
      if (outcome.kind === "approvalRequired") {
        approval.value = outcome.risk;
        acknowledged.value = false;
        return;
      }

      approval.value = null;
      lastSent.value += 1;
      // The values that were used are the ones worth offering next time.
      const current = draft.value;
      if (current?.id && slots.value.length > 0) {
        await rememberValues(current.id);
      }
      await load();
    } catch (error) {
      sendFailure.value = asShellError(error, "command_send");
    } finally {
      sending.value = false;
    }
  }

  /** Moves the values just used to the front of their slot's history, and stores that. */
  async function rememberValues(id: string): Promise<void> {
    const current = draft.value;
    const stored = groups.value.flatMap((group) => group.commands).find((entry) => entry.id === id);
    if (!current || !stored) {
      return;
    }

    const history: string[][] = Array.from({ length: SLOTS }, (_, slot) => [...(stored.slotHistory[slot] ?? [])]);
    for (const slot of slots.value) {
      const value = values.value[slot - 1]?.trim() ?? "";
      if (value === "") {
        continue;
      }
      const valuesForSlot = history[slot - 1] ?? [];
      history[slot - 1] = [value, ...valuesForSlot.filter((existing) => existing !== value)].slice(0, 12);
    }

    await backend.commands.save({ ...stored, id, slotHistory: history });
  }

  return {
    groups,
    history,
    loaded,
    failure,
    draft,
    values,
    target,
    approval,
    acknowledged,
    sending,
    sendFailure,
    lastSent,
    slots,
    filled,
    load,
    edit,
    startNew,
    closeDraft,
    setValue,
    save,
    remove,
    addGroup,
    removeGroup,
    clearHistory,
    editFromHistory,
    send,
  };
});
