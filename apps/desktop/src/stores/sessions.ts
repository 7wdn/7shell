/**
 * The saved sessions and the groups they sit in.
 *
 * Two rules shape this store:
 *
 * 1. **It is the only reader of the library.** The sidebar list and the count on the home view
 *    come from the same array, so they cannot disagree about what is saved.
 * 2. **A failure is shown where the person who caused it is looking.** `load`, `remove` and the
 *    group actions are shell actions, and record their envelope so the sidebar can show it; `save`
 *    belongs to the editor dialog, so it rejects and leaves the envelope to the dialog, which is
 *    already open.
 *
 * The tree the sidebar draws is computed here rather than in the sidebar, because three other
 * things ask about it: the menus name groups by their path, a drop has to know what is under a row,
 * and a move has to expand what it landed in. Which rows are *closed* is the one piece of pure
 * appearance this store holds; it lives in memory, so a restart draws the tree open.
 *
 * The search and the filters live here too, for the same reason: they decide which rows exist, and
 * a second copy of that decision in a component is how the two come apart. They are a view over
 * `sessions` and `groups` — the arrays themselves are never filtered — so everything that writes
 * (a move, a removal, an import) works on the whole library while the list is narrowed.
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import { type SessionFilter, filterIsOn, filterLibrary } from "@/lib/sessions/filter";
import {
  type Drop,
  type DropZone,
  type GroupPlacement,
  type ParentId,
  nextOrder,
  planGroupDrop,
  planSessionDrop,
  planSiblingMove,
} from "@/lib/sessions/groups";
import { type TreeRow, visibleRows } from "@/lib/sessions/tree";
import type { AuthMethod } from "@/types/generated/protocols";
import type { Session, SessionExportSummary, SessionGroup, SessionImportSummary, SecretInput } from "@/types/session";

export type LibraryStatus = "idle" | "loading" | "ready" | "failed";

export const useSessionsStore = defineStore("sessions", () => {
  const sessions = ref<Session[]>([]);
  const groups = ref<SessionGroup[]>([]);
  /** The groups drawn closed, by id. In memory: this is how the list looks, not what is saved. */
  const collapsed = ref<ReadonlySet<string>>(new Set());
  /** What the list is narrowed to. Empty means everything, which is how it starts. */
  const filter = ref<SessionFilter>({ query: "", methods: new Set() });
  const status = ref<LibraryStatus>("idle");
  const failure = ref<ShellError | null>(null);

  /** Whether the list is narrowed at all, which is what its "nothing found" state keys off. */
  const filtering = computed(() => filterIsOn(filter.value));

  /** The library as the filter leaves it, with the groups that have to be drawn open. */
  const shown = computed(() => filterLibrary(groups.value, sessions.value, filter.value));

  /** The rows the sidebar draws, in reading order, with closed groups left closed. */
  const rows = computed<TreeRow[]>(() =>
    visibleRows(shown.value.groups, shown.value.sessions, collapsed.value, shown.value.revealed),
  );

  function setQuery(query: string): void {
    filter.value = { ...filter.value, query };
  }

  /** Adds a method to the filter, or takes it out again when it is already in. */
  function toggleMethod(method: AuthMethod): void {
    const methods = new Set(filter.value.methods);
    if (!methods.delete(method)) {
      methods.add(method);
    }
    filter.value = { ...filter.value, methods };
  }

  function clearFilter(): void {
    filter.value = { query: "", methods: new Set() };
  }

  async function load(): Promise<void> {
    status.value = "loading";
    failure.value = null;
    try {
      const [list, groupList] = await Promise.all([backend.sessions.list(), backend.sessions.groups.list()]);
      sessions.value = list;
      groups.value = groupList;
      status.value = "ready";
    } catch (error) {
      failure.value = asShellError(error, "sessions_list");
      status.value = "failed";
    }
  }

  /** Saves a session and keeps the backend's answer, which is where `hasSecret` comes from. */
  async function save(session: Session, secret?: SecretInput): Promise<Session> {
    failure.value = null;
    const saved = await backend.sessions.save(session, secret);
    const index = sessions.value.findIndex((entry) => entry.id === saved.id);
    if (index >= 0) {
      sessions.value[index] = saved;
    } else {
      sessions.value.push(saved);
    }
    return saved;
  }

  async function remove(id: string): Promise<void> {
    failure.value = null;
    try {
      await backend.sessions.remove(id);
      sessions.value = sessions.value.filter((session) => session.id !== id);
    } catch (error) {
      failure.value = asShellError(error, "session_delete");
    }
  }

  /**
   * Writes the library to a file the user picks, in the shell.
   *
   * `null` is "nothing was written": either the file dialog was dismissed, or the write failed and
   * the envelope is in `failure`. There is no way to tell those apart from the answer, and there
   * does not need to be — one is somebody changing their mind and the other is shown where they are
   * looking.
   */
  async function exportLibrary(
    title: string,
    fileName: string,
    includeSecrets: boolean,
  ): Promise<SessionExportSummary | null> {
    failure.value = null;
    try {
      return (await backend.sessions.export(title, fileName, includeSecrets)) ?? null;
    } catch (error) {
      failure.value = asShellError(error, "sessions_export");
      return null;
    }
  }

  /**
   * Adds what a file holds, in the shell, and reads the library again afterwards.
   *
   * Read again rather than merged here: which ids the import had to replace is the backend's
   * answer, and a second implementation of that rule on this side is how the list and the vault
   * come to disagree about what is saved.
   */
  async function importLibrary(title: string): Promise<SessionImportSummary | null> {
    failure.value = null;
    try {
      const summary = await backend.sessions.import(title);
      if (!summary) {
        return null;
      }
      await load();
      return summary;
    } catch (error) {
      failure.value = asShellError(error, "sessions_import");
      return null;
    }
  }

  /**
   * Writes a group. A group that is already saved keeps every field the caller did not set, which
   * is why this takes the whole group rather than a patch.
   */
  async function saveGroup(group: SessionGroup): Promise<boolean> {
    failure.value = null;
    try {
      const saved = await backend.sessions.groups.save(group);
      const index = groups.value.findIndex((entry) => entry.id === saved.id);
      if (index >= 0) {
        groups.value[index] = saved;
      } else {
        groups.value.push(saved);
      }
      return true;
    } catch (error) {
      failure.value = asShellError(error, "group_save");
      return false;
    }
  }

  /**
   * Makes a group with a fresh id: at the root unless a parent is named, and after the groups that
   * already sit there. A group made inside a closed one opens it — a group that was just made and
   * cannot be seen is a group that was not made.
   */
  async function addGroup(name: string, parentId: ParentId = null): Promise<boolean> {
    const worked = await saveGroup({
      id: crypto.randomUUID(),
      name,
      parentId: parentId ?? undefined,
      order: nextOrder(groups.value, parentId),
    });
    if (worked) {
      reveal(parentId);
    }
    return worked;
  }

  /**
   * Removes a group and nothing else. The backend moves what was inside it out to the root, and
   * this does the same to the copy on screen, so the list is right without reading it all again —
   * a session that was in the group stays where somebody can see it, under Ungrouped.
   */
  async function removeGroup(id: string): Promise<boolean> {
    failure.value = null;
    try {
      await backend.sessions.groups.remove(id);
      groups.value = groups.value
        .filter((group) => group.id !== id)
        .map((group) => (group.parentId === id ? { ...group, parentId: undefined } : group));
      sessions.value = sessions.value.map((session) =>
        session.groupId === id ? { ...session, groupId: undefined } : session,
      );
      forget([id]);
      return true;
    } catch (error) {
      failure.value = asShellError(error, "group_delete");
      return false;
    }
  }

  /** Puts one group a place up or down among the groups it shares a parent with. */
  async function moveGroupBy(id: string, direction: -1 | 1): Promise<boolean> {
    const placements = planSiblingMove(groups.value, id, direction);
    return placements !== null && write(placements);
  }

  /**
   * Puts a group where a drop said it goes: into another group, beside one, or at the root.
   *
   * Every group whose parent or order changes is written, one after another, and a refusal stops
   * the rest with its envelope in `failure`. The writes are deliberately not one transaction: the
   * vault has no way to say "all of these or none", and a half-finished move is still a tree that
   * can be drawn — so the honest thing is to say what happened rather than to pretend. Dropping
   * into a group opens it, so what was just put there can be seen.
   */
  async function placeGroup(id: string, drop: Drop, zone: DropZone = "inside"): Promise<boolean> {
    const placements = planGroupDrop(groups.value, id, drop, zone);
    if (placements === null || !(await write(placements))) {
      return false;
    }
    if (drop.kind === "group" && zone === "inside") {
      reveal(drop.id);
    }
    return true;
  }

  /**
   * Puts a session into a group, or into none, without opening the editor: a group is one field of
   * a session, and it is the whole session that gets written. No secret travels — an absent one
   * means "keep the stored one", the same rule the editor follows.
   */
  async function placeSession(id: string, drop: Drop): Promise<boolean> {
    const session = sessions.value.find((entry) => entry.id === id);
    if (!session) {
      return false;
    }
    const plan = planSessionDrop(groups.value, session, drop);
    if (!plan) {
      return false;
    }

    failure.value = null;
    try {
      const saved = await backend.sessions.save(
        { ...session, groupId: plan.groupId ?? undefined, updatedAt: Math.floor(Date.now() / 1000) },
        undefined,
      );
      const index = sessions.value.findIndex((entry) => entry.id === saved.id);
      if (index >= 0) {
        sessions.value[index] = saved;
      } else {
        sessions.value.push(saved);
      }
      reveal(plan.groupId);
      return true;
    } catch (error) {
      failure.value = asShellError(error, "session_save");
      return false;
    }
  }

  /**
   * Whether a drop would do anything, which is what the drag's indicator is drawn from.
   *
   * The same planners the drops themselves call: an indicator that promised something the release
   * would refuse is the one thing a drag must not do, and two answers to "would this work" is how
   * that happens.
   */
  function canPlaceGroup(id: string, drop: Drop, zone: DropZone): boolean {
    return planGroupDrop(groups.value, id, drop, zone) !== null;
  }

  function canPlaceSession(id: string, drop: Drop): boolean {
    const session = sessions.value.find((entry) => entry.id === id);
    return session !== undefined && planSessionDrop(groups.value, session, drop) !== null;
  }

  /** Writes each placement, one group at a time, stopping at the first refusal. */
  async function write(placements: readonly GroupPlacement[]): Promise<boolean> {
    for (const placement of placements) {
      const group = groups.value.find((entry) => entry.id === placement.id);
      if (
        !group ||
        !(await saveGroup({ ...group, parentId: placement.parentId ?? undefined, order: placement.order }))
      ) {
        return false;
      }
    }
    return true;
  }

  /** Opens a group, if it is closed: where something was just put has to be visible. */
  function reveal(parentId: ParentId): void {
    if (parentId === null || !collapsed.value.has(parentId)) {
      return;
    }
    const next = new Set(collapsed.value);
    next.delete(parentId);
    collapsed.value = next;
  }

  /** Forgets the closed state of groups that are gone, so the set does not grow forever. */
  function forget(ids: readonly string[]): void {
    const next = new Set([...collapsed.value].filter((id) => !ids.includes(id)));
    collapsed.value = next;
  }

  /** Opens or closes one group. */
  function setCollapsed(id: string, closed: boolean): void {
    const next = new Set(collapsed.value);
    if (closed) {
      next.add(id);
    } else {
      next.delete(id);
    }
    collapsed.value = next;
  }

  return {
    sessions,
    groups,
    collapsed,
    filter,
    filtering,
    status,
    failure,
    rows,
    load,
    save,
    remove,
    saveGroup,
    addGroup,
    removeGroup,
    moveGroupBy,
    placeGroup,
    placeSession,
    canPlaceGroup,
    canPlaceSession,
    setCollapsed,
    setQuery,
    toggleMethod,
    clearFilter,
    exportLibrary,
    importLibrary,
  };
});
