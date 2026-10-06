/**
 * What the list shows while somebody is looking for one thing among many.
 *
 * A filter is a *view* and never a change: nothing here writes, and the library itself is left
 * exactly as it was. Two rules make the result predictable, and both are about what a person
 * hunting through a list expects to see:
 *
 * 1. **A group is drawn when something inside it is** — a heading with nothing under it is noise
 *    when somebody is searching. A group whose own name matches is drawn too, with what is inside
 *    it: somebody who typed a folder's name is asking to see that folder.
 * 2. **A group that is drawn is drawn open.** A match inside a closed folder is a match nobody can
 *    see, which reads as "nothing was found" — the one answer a search must never give wrongly.
 *
 * Matching is by terms, all of which have to appear somewhere in the session's own text: `"prod
 * db"` finds the database in the production group and not the web server beside it. Text is matched
 * against what a person would type to identify the machine — the name, the address, the user, the
 * port, the protocol — and never against the password, which is not here to match in the first
 * place.
 */
import type { AuthMethod } from "@/types/generated/protocols";
import type { Session, SessionGroup } from "@/types/session";

import { childrenOf, effectiveParentId, type ParentId, sessionParentId } from "./groups";

/** What the person is looking for. An empty query and no methods is "everything". */
export interface SessionFilter {
  readonly query: string;
  /** Authentication methods to keep; empty means every method. */
  readonly methods: ReadonlySet<AuthMethod>;
}

/** The filter that keeps everything. */
export const NO_FILTER: SessionFilter = { query: "", methods: new Set() };

/** Whether anything is being kept out at all. */
export function filterIsOn(filter: SessionFilter): boolean {
  return filter.query.trim() !== "" || filter.methods.size > 0;
}

/** Whether a session passes the method half of the filter. */
export function passesMethods(session: Session, methods: ReadonlySet<AuthMethod>): boolean {
  return methods.size === 0 || methods.has(session.auth.method);
}

/**
 * Whether every term appears somewhere in what identifies this session.
 *
 * A term search rather than a single substring, because the two things people type together —
 * "prod" and "db" — describe different fields, and requiring one exact run of characters would
 * find neither.
 */
export function matchesQuery(session: Session, query: string): boolean {
  return terms(query).every((term) => haystack(session).includes(term));
}

/** The library as the list should draw it: what is kept, and what has to be drawn open. */
export interface FilteredLibrary {
  readonly sessions: readonly Session[];
  readonly groups: readonly SessionGroup[];
  /** The groups to draw open whatever their saved state is: the ones on the way to a match. */
  readonly revealed: ReadonlySet<string>;
}

/** The sessions, groups and open folders to draw for one filter. */
export function filterLibrary(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  filter: SessionFilter,
): FilteredLibrary {
  if (!filterIsOn(filter)) {
    return { sessions, groups, revealed: new Set() };
  }

  const query = filter.query.trim().toLowerCase();
  // Only a real query can name a group: an empty one is a substring of every name, which would
  // draw every empty folder in the library the moment a method is filtered on.
  const named =
    query === ""
      ? new Set<string>()
      : new Set(groups.filter((group) => group.name.toLowerCase().includes(query)).map((group) => group.id));

  const keptSessions = sessions.filter(
    (session) =>
      passesMethods(session, filter.methods) &&
      (matchesQuery(session, query) || (query !== "" && isInsideANamedGroup(groups, session, named))),
  );

  const drawn = new Set<string>();
  const visit = (parentId: ParentId): boolean => {
    let holdsSomething = false;
    for (const group of childrenOf(groups, parentId)) {
      const inside = visit(group.id);
      const mine = keptSessions.some((session) => sessionParentId(groups, session) === group.id);
      if (inside || mine || named.has(group.id)) {
        drawn.add(group.id);
        holdsSomething = true;
      }
    }
    return holdsSomething;
  };
  visit(null);

  return {
    sessions: keptSessions,
    groups: groups.filter((group) => drawn.has(group.id)),
    // Everything that is drawn is drawn open: see the rule at the top of this file.
    revealed: drawn,
  };
}

/** One term per word, lowercased, blanks dropped. */
function terms(query: string): readonly string[] {
  return query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter((term) => term !== "");
}

/** Everything about a session somebody might type to find it. */
function haystack(session: Session): string {
  return [
    session.name,
    session.host,
    session.user,
    `${session.user}@${session.host}:${session.port}`,
    String(session.port),
    session.protocol,
  ]
    .join(" ")
    .toLowerCase();
}

/** Whether a group or any group above it matched by name. */
function isInsideANamedGroup(groups: readonly SessionGroup[], session: Session, named: ReadonlySet<string>): boolean {
  let parent = sessionParentId(groups, session);
  const seen = new Set<string>();
  while (parent !== null && !seen.has(parent)) {
    if (named.has(parent)) {
      return true;
    }
    seen.add(parent);
    parent = effectiveParentId(groups, parent);
  }
  return false;
}
