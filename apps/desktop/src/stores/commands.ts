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
import { BUILTIN_COMMANDS, libraryIsShipped } from "@/lib/commands/builtin";
import { DEFAULT_GROUP_ID } from "@/lib/commands/defaults";
import { fillSlots, slotsIn, SLOTS } from "@/lib/commands/slots";
import { useTerminalsStore } from "@/stores/terminals";
import type { RiskAssessment } from "@/types/batch";
import type { CommandHistoryEntry, SendFailure, Snippet, SnippetGroup } from "@/types/snippet";

// What a slot is lives in `lib/commands/slots.ts`, because the completion menu fills the same lines
// the panel sends; these names stay here so a caller that has the store in hand needs one import.
export { SLOTS, fillSlots, slotsIn };

/** A draft being edited: the fields the form owns, before it is a saved command. */
export interface SnippetDraft {
  id: string;
  groupId: string;
  name: string;
  /** The short text that brings it up in the command line; empty means none. */
  prefix: string;
  command: string;
  appendCr: boolean;
  /** Whether the completion offers it. */
  enabled: boolean;
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
  /** Why the send itself could not be made at all, rather than not made to every terminal. */
  const sendFailure = ref<ShellError | null>(null);
  /** The terminals the last send could not reach, each with the reason a person reads. */
  const sendFailures = ref<SendFailure[]>([]);
  const lastSent = ref(0);

  const slots = computed(() => (draft.value ? slotsIn(draft.value.command) : []));
  const filled = computed(() => (draft.value ? fillSlots(draft.value.command, values.value) : ""));

  /**
   * Fills a library that holds none of the commands this build ships with the ones it ships.
   *
   * "Holds none of them" rather than "is empty": somebody who has been using the application for a
   * while has lines of their own in it, and a shipped list that only ever reached an empty library
   * would never reach them at all. A library that already has one of these keeps what it has — which
   * is the whole of what makes deleting one stick, since a list that is put back on every read is a
   * list nobody can prune. What counts as having one is its name: a name is what somebody reads, and
   * it is what they would have changed if they meant to keep it under another word.
   *
   * They arrive as ordinary commands: editable, switchable, deletable. Nothing marks them as ours,
   * which is what lets them be treated like anything else.
   */
  async function seedBuiltins(): Promise<void> {
    const group = groups.value.find((entry) => entry.id === DEFAULT_GROUP_ID);
    const names = new Set(groups.value.flatMap((entry) => entry.commands.map((command) => command.name)));
    if (group === undefined || BUILTIN_COMMANDS.some((builtin) => names.has(builtin.name))) {
      return;
    }

    for (const builtin of BUILTIN_COMMANDS) {
      await backend.commands.save({
        id: "",
        groupId: group.id,
        name: builtin.name,
        prefix: builtin.prefix,
        command: builtin.command,
        appendCr: true,
        enabled: true,
        order: 0,
        slotHistory: [],
      });
    }
    groups.value = await backend.commands.library();
  }

  async function load(): Promise<void> {
    try {
      groups.value = await backend.commands.library();
      history.value = await backend.commands.history();
      failure.value = null;
      loaded.value = true;
      await seedBuiltins();
    } catch (error) {
      failure.value = asShellError(error, "snippets_open");
    }
  }

  function edit(snippet: Snippet): void {
    draft.value = {
      id: snippet.id,
      groupId: snippet.groupId,
      name: snippet.name,
      prefix: snippet.prefix,
      command: snippet.command,
      appendCr: snippet.appendCr,
      enabled: snippet.enabled,
    };
    approval.value = null;
    acknowledged.value = false;
    // The last value used for each slot, which is what somebody changing one thing expects to find.
    values.value = Array.from({ length: SLOTS }, (_, slot) => snippet.slotHistory[slot]?.[0] ?? "");
  }

  function startNew(groupId: string): void {
    draft.value = { id: "", groupId, name: "", prefix: "", command: "", appendCr: true, enabled: true };
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
        prefix: current.prefix.trim(),
        command: current.command,
        appendCr: current.appendCr,
        enabled: current.enabled,
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

  /**
   * Moves a saved command one place up or down inside its group.
   *
   * The order is stored rather than implied, so this is two writes with swapped values — the
   * neighbour and the command — rather than a list that is renumbered on every change.
   */
  async function move(id: string, direction: -1 | 1): Promise<void> {
    const group = groups.value.find((entry) => entry.commands.some((command) => command.id === id));
    if (!group) {
      return;
    }

    const index = group.commands.findIndex((command) => command.id === id);
    const neighbour = group.commands[index + direction];
    const moving = group.commands[index];
    if (!moving || !neighbour) {
      return;
    }

    try {
      await backend.commands.save({ ...neighbour, order: moving.order });
      await backend.commands.save({ ...moving, order: neighbour.order });
      await load();
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

  /**
   * Renames a group.
   *
   * The same call that creates one, with the id it already has: the commands inside are not part of
   * a rename, and the storage keeps them for exactly that reason.
   */
  async function renameGroup(id: string, name: string): Promise<void> {
    const group = groups.value.find((entry) => entry.id === id);
    if (!group || name.trim() === "") {
      return;
    }

    try {
      await backend.commands.saveGroup({ ...group, name: name.trim(), commands: [] });
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

  /**
   * Switches one command off or on.
   *
   * A write of the command rather than a flag kept beside it: what the library holds is what is
   * stored, and a switch that lived somewhere else would be a second answer to what the list says.
   */
  async function setEnabled(snippet: Snippet, enabled: boolean): Promise<void> {
    try {
      await backend.commands.save({ ...snippet, enabled });
      await load();
      failure.value = null;
    } catch (error) {
      failure.value = asShellError(error, "snippet_save");
    }
  }

  /**
   * Puts the library back to what this build ships.
   *
   * Everything goes: the commands somebody wrote, the groups they made, the values remembered for a
   * shipped command's slots. Then the ordinary first-install rule writes the shipped list back —
   * which is the same list, put back the same way, so there is only one answer to what a fresh
   * library holds. The history is not touched: that is what was typed, not what is kept, and it is
   * cleared from where it is drawn.
   */
  async function restoreDefaults(): Promise<void> {
    try {
      for (const command of groups.value.flatMap((group) => group.commands)) {
        await backend.commands.delete(command.id);
      }
      for (const group of groups.value) {
        if (group.id !== DEFAULT_GROUP_ID) {
          await backend.commands.deleteGroup(group.id);
        }
      }
      await load();
    } catch (error) {
      failure.value = asShellError(error, "snippet_delete");
    }
  }

  /** Whether the library is anything other than what this build ships. */
  const needsRestore = computed(() => !libraryIsShipped(groups.value));

  /** Puts a line from the history into the editor, without slots of its own to fill. */
  function editFromHistory(entry: CommandHistoryEntry): void {
    draft.value = {
      id: "",
      groupId: groups.value[0]?.id ?? "default",
      name: entry.command.slice(0, 40),
      prefix: "",
      command: entry.command,
      appendCr: true,
      enabled: true,
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
      // Per terminal: "it reached three of them" is the useful answer when one tab has ended.
      sendFailures.value = [...outcome.report.failures];
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

  /**
   * Sends one line, typed rather than edited: what the terminal's input bar does.
   *
   * The target is the terminal on screen, because the bar lives in it — and a line sent to the one
   * shell somebody is looking at is not gated, which is the same rule as typing it there.
   */
  async function sendLiteral(line: string): Promise<void> {
    const terminals = useTerminalsStore();
    const handle = terminals.activeTerminalHandle();
    if (handle === null || line.trim() === "" || sending.value) {
      return;
    }

    sending.value = true;
    sendFailure.value = null;
    try {
      const outcome = await backend.commands.send({ kind: "terminals", handles: [handle] }, line, true, false);
      if (outcome.kind === "approvalRequired") {
        // The core does not gate a single terminal, so this cannot arrive today. Reported rather
        // than ignored: a silent no-op here would be a line that never ran and never said why.
        approval.value = outcome.risk;
        return;
      }

      sendFailures.value = [...outcome.report.failures];
      lastSent.value += 1;
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
    sendFailures,
    lastSent,
    slots,
    filled,
    load,
    edit,
    startNew,
    closeDraft,
    setValue,
    save,
    move,
    remove,
    addGroup,
    renameGroup,
    removeGroup,
    clearHistory,
    editFromHistory,
    setEnabled,
    restoreDefaults,
    needsRestore,
    send,
    sendLiteral,
  };
});
