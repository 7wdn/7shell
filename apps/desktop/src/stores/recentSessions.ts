/**
 * The sessions used last, shared by whatever draws a short list of them.
 *
 * A store because two things ask it questions at once: the opens that write it down, and the home
 * page that ranks the library by it. The rules — the cap, the order, what a stored value that no
 * longer parses means — are in `lib/sessions/recent`, which is what the tests exercise directly.
 */

import { defineStore } from "pinia";
import { ref } from "vue";

import {
  MAX_RECENT_SESSIONS,
  rankRecentSessions,
  readRecentSessionIds,
  recordRecentSession,
  writeRecentSessionIds,
} from "@/lib/sessions/recent";
import type { Session } from "@/types/session";

export const useRecentSessionsStore = defineStore("recentSessions", () => {
  const ids = ref<string[]>([]);

  /** Reads what was remembered on this machine, once, when the window opens. */
  function load(): void {
    ids.value = readRecentSessionIds();
  }

  /**
   * Writes down that a session is being used, which moves it to the front of the list.
   *
   * Called when an open is *asked for* rather than when it succeeds: a host key still to be decided
   * is somebody using that session, and an attempt that failed is still the session they reached
   * for — the alternative would be a list of what happened to work.
   */
  function record(sessionId: string): void {
    const next = recordRecentSession(ids.value, sessionId);
    if (next === ids.value) {
      return;
    }
    ids.value = next;
    writeRecentSessionIds(next);
  }

  /** What the home page draws: most recently used first, filled out from the library. */
  function ranked(sessions: readonly Session[], limit: number = MAX_RECENT_SESSIONS): Session[] {
    return rankRecentSessions(sessions, ids.value, limit);
  }

  return { ids, load, record, ranked };
});
