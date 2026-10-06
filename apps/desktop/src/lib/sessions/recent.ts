/**
 * Which sessions were used last.
 *
 * This is usage memory, not a setting, which is why it is not in the settings file: that file
 * records choices somebody made, and it can be put back to the defaults. "The last few hosts I
 * opened" is not a choice and has no default — putting it there would mean a restore that quietly
 * forgets it, or a field the settings page would have to explain and could not.
 *
 * It lives in the WebView's own storage, which is also the right durability: losing it costs a list
 * that falls back to the library's own order, and nothing else in the application reads it.
 *
 * A session that has been deleted stays in the stored ids until the cap pushes it out — nothing
 * here knows the library — so ranking drops ids the library no longer has rather than drawing a row
 * that would open nothing.
 */

/** Where the ids live. Versioned, because a list read by a later shape is worth discarding. */
export const RECENT_SESSIONS_KEY = "sevenshell-recent-session-ids-v1";

/** How many are remembered, and how many the quick-start list draws. */
export const MAX_RECENT_SESSIONS = 5;

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    // A WebView with storage refused throws on the property itself rather than on a call, and a
    // convenience list is not worth failing a render over.
    return null;
  }
}

/** The list's own rules, applied to whatever was read: strings, no repeats, at most the cap. */
function normalizeRecentSessionIds(values: readonly unknown[]): string[] {
  const ids: string[] = [];
  const seen = new Set<string>();

  for (const value of values) {
    if (typeof value !== "string") {
      continue;
    }
    const id = value.trim();
    if (id === "" || seen.has(id)) {
      continue;
    }
    seen.add(id);
    ids.push(id);
    if (ids.length === MAX_RECENT_SESSIONS) {
      break;
    }
  }

  return ids;
}

/** What was stored, if it still parses into a list. Anything else reads as "nothing remembered". */
export function parseRecentSessionIds(raw: string | null): string[] {
  if (!raw) {
    return [];
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return [];
  }

  return Array.isArray(parsed) ? normalizeRecentSessionIds(parsed) : [];
}

/**
 * The list after one more use: the session moves to the front and everything else shifts down.
 *
 * The same array comes back when nothing moved, so a caller can tell "nothing changed" without
 * comparing contents — the common case being somebody reopening the session they just had.
 */
export function recordRecentSession(history: readonly string[], sessionId: string): string[] {
  const id = sessionId.trim();
  const next = normalizeRecentSessionIds(id === "" ? history : [id, ...history]);
  const unchanged = next.length === history.length && next.every((value, index) => value === history[index]);
  return unchanged ? (history as string[]) : next;
}

/**
 * What the quick-start list draws: the remembered sessions in the order they were used, then the
 * rest of the library to fill the list out.
 *
 * The filling is deliberate rather than a degradation: a fresh install has nothing remembered, and
 * a card that stayed empty until somebody had used something would teach nothing. Remembered ids
 * that no longer exist are skipped rather than counted against the limit.
 */
export function rankRecentSessions<T extends { id: string }>(
  sessions: readonly T[],
  history: readonly string[],
  limit: number = MAX_RECENT_SESSIONS,
): T[] {
  const capped = Number.isFinite(limit)
    ? Math.max(0, Math.min(Math.floor(limit), MAX_RECENT_SESSIONS))
    : MAX_RECENT_SESSIONS;
  if (capped === 0) {
    return [];
  }

  const byId = new Map(sessions.map((session) => [session.id, session]));
  const ranked: T[] = [];
  const taken = new Set<string>();

  for (const id of normalizeRecentSessionIds(history)) {
    const session = byId.get(id);
    if (!session) {
      continue;
    }
    ranked.push(session);
    taken.add(id);
    if (ranked.length === capped) {
      return ranked;
    }
  }

  for (const session of sessions) {
    if (taken.has(session.id)) {
      continue;
    }
    ranked.push(session);
    taken.add(session.id);
    if (ranked.length === capped) {
      break;
    }
  }

  return ranked;
}

/** What is remembered on this machine. A list that cannot be read is an empty one, not an error. */
export function readRecentSessionIds(): string[] {
  const store = storage();
  if (!store) {
    return [];
  }
  try {
    return parseRecentSessionIds(store.getItem(RECENT_SESSIONS_KEY));
  } catch {
    return [];
  }
}

/** Writes the list down. A refusal is dropped for the same reason a missing store is. */
export function writeRecentSessionIds(ids: readonly string[]): void {
  const store = storage();
  if (!store) {
    return;
  }
  try {
    store.setItem(RECENT_SESSIONS_KEY, JSON.stringify(ids));
  } catch {
    // Full, or refused. The list is a convenience, and the next use tries again.
  }
}
