/**
 * The saved sessions and the groups they sit in.
 *
 * Two rules shape this store:
 *
 * 1. **It is the only reader of the library.** The sidebar list and the count on the home view
 *    come from the same array, so they cannot disagree about what is saved.
 * 2. **A failure is shown where the person who caused it is looking.** `load` and `remove` are
 *    shell actions, and record their envelope so the sidebar can show it; `save` belongs to the
 *    editor dialog, so it rejects and leaves the envelope to the dialog, which is already open.
 */

import { defineStore } from "pinia";
import { computed, ref } from "vue";

import { backend } from "@/lib/backend/api";
import { type ShellError, asShellError } from "@/lib/backend/errors";
import type { Session, SessionGroup, SecretInput } from "@/types/session";

export type LibraryStatus = "idle" | "loading" | "ready" | "failed";

/** One heading in the sidebar: a named group, or the sessions that are in none. */
export interface SessionSection {
  readonly key: string;
  /** Absent for the sessions that belong to no group. */
  readonly name: string | null;
  readonly sessions: readonly Session[];
}

export const useSessionsStore = defineStore("sessions", () => {
  const sessions = ref<Session[]>([]);
  const groups = ref<SessionGroup[]>([]);
  const status = ref<LibraryStatus>("idle");
  const failure = ref<ShellError | null>(null);

  /**
   * The list as the sidebar draws it: every group in its own order, then everything that has no
   * group — including a session whose group is no longer saved, which must not vanish silently.
   */
  const sections = computed<SessionSection[]>(() => {
    const ordered = [...groups.value].sort((left, right) => left.order - right.order);
    const known = new Set(ordered.map((group) => group.id));
    const built: SessionSection[] = ordered.map((group) => ({
      key: group.id,
      name: group.name,
      sessions: sessions.value.filter((session) => session.groupId === group.id),
    }));

    const loose = sessions.value.filter((session) => !session.groupId || !known.has(session.groupId));
    if (loose.length > 0) {
      built.push({ key: "ungrouped", name: null, sessions: loose });
    }
    return built;
  });

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

  return { sessions, groups, status, failure, sections, load, save, remove };
});
