/**
 * What the list shows while somebody is looking for one thing among many.
 *
 * A filter is a *view* and never a change: nothing here writes, and the library itself is left
 * exactly as it was. Three rules make the result predictable, and all three are about what a person
 * hunting through a list expects to see:
 *
 * 1. **A group is drawn when something inside it is** — a heading with nothing under it is noise
 *    when somebody is searching. A group whose own name matches is drawn too, with what is inside
 *    it: somebody who typed a folder's name is asking to see that folder.
 * 2. **A group that is drawn is drawn open.** A match inside a closed folder is a match nobody can
 *    see, which reads as "nothing was found" — the one answer a search must never give wrongly.
 * 3. **A query that matches nothing draws nothing.** Not an empty state, not a sentence, not a
 *    button: a list that says "nothing matched" is a list that has decided what the person meant,
 *    and the way back is already in front of them — the query they just typed, with a cross in it.
 *
 * Matching is by terms, all of which have to appear somewhere in the session's own text: `"prod
 * db"` finds the database in the production group and not the web server beside it. Text is matched
 * against what a person would type to identify the machine — the name, the address, the user, the
 * port, the protocol — and never against the password, which is not here to match in the first
 * place. A query can also be a regular expression, which is the same matching one level up: the
 * whole pattern against the same text, rather than each term in it.
 */
import type { AuthMethod } from "@/types/generated/protocols";
import type { Session, SessionGroup } from "@/types/session";

import { childrenOf, effectiveParentId, type ParentId, sessionParentId } from "./groups";

/** What the person is looking for. An empty query and no switches set is "everything". */
export interface SessionFilter {
  readonly query: string;
  /** Authentication methods to keep; empty means every method. */
  readonly methods: ReadonlySet<AuthMethod>;
  /** The query is a JavaScript regular expression rather than a set of words to look for. */
  readonly regex: boolean;
  /** Keep only the sessions something is open for right now. */
  readonly onlyOpen: boolean;
}

/** The filter that keeps everything. */
export const NO_FILTER: SessionFilter = { query: "", methods: new Set(), regex: false, onlyOpen: false };

/** Whether anything is being kept out at all. */
export function filterIsOn(filter: SessionFilter): boolean {
  return filter.query.trim() !== "" || filter.methods.size > 0 || filter.onlyOpen;
}

/** Whether a session passes the method half of the filter. */
export function passesMethods(session: Session, methods: ReadonlySet<AuthMethod>): boolean {
  return methods.size === 0 || methods.has(session.auth.method);
}

/**
 * Whether a query matches what identifies a session.
 *
 * Two readings of the same text, chosen by the person: words that all have to appear, or one
 * regular expression. A pattern that does not compile matches nothing and says so through
 * [`regexIsInvalid`], which is what the search box draws its own state from — a search that is
 * silently ignored is worse than one that is visibly broken.
 */
export function matchesQuery(session: Session, query: string, regex = false): boolean {
  return sessionMatcher(textMatcher(query, regex), regex)(session);
}

/** Whether the query is a regular expression that does not compile. */
export function regexIsInvalid(query: string): boolean {
  return compileRegex(query) === null;
}

/** The library as the list should draw it: what is kept, and what has to be drawn open. */
export interface FilteredLibrary {
  readonly sessions: readonly Session[];
  readonly groups: readonly SessionGroup[];
  /** The groups to draw open whatever their saved state is: the ones on the way to a match. */
  readonly revealed: ReadonlySet<string>;
}

/**
 * The sessions, groups and open folders to draw for one filter.
 *
 * `open` is the sessions something is open for right now, which only the switches section asks
 * about. It is passed in rather than looked up because what is open is not part of the library: the
 * vault holds what is saved, and a terminal being up is a fact about this run.
 */
export function filterLibrary(
  groups: readonly SessionGroup[],
  sessions: readonly Session[],
  filter: SessionFilter,
  open: ReadonlySet<string> = new Set(),
): FilteredLibrary {
  if (!filterIsOn(filter)) {
    return { sessions, groups, revealed: new Set() };
  }

  const query = filter.query.trim();
  const matches = textMatcher(filter.query, filter.regex);
  // Only a real query can name a group: an empty one is a substring of every name, which would
  // draw every empty folder in the library the moment a method is filtered on.
  const named =
    query === "" ? new Set<string>() : new Set(groups.filter((group) => matches(group.name)).map((group) => group.id));

  const matchesSession = sessionMatcher(matches, filter.regex);
  const keptSessions = sessions.filter(
    (session) =>
      passesMethods(session, filter.methods) &&
      (!filter.onlyOpen || open.has(session.id)) &&
      (matchesSession(session) || (query !== "" && isInsideANamedGroup(groups, session, named))),
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

/**
 * How the text half of the filter reads a string, compiled once for the whole pass.
 *
 * The regular expression is matched case-insensitively, like the plain search is: JavaScript has no
 * way to say "ignore case" inside a pattern, so leaving it case-sensitive would give a person who
 * writes `Prod` a search that misses `prod` and no way to fix it.
 */
function textMatcher(query: string, regex: boolean): (subject: string) => boolean {
  if (regex) {
    const pattern = compileRegex(query);
    return (subject) => pattern !== null && pattern.test(subject);
  }
  const wanted = terms(query);
  return (subject) => wanted.every((term) => subject.toLowerCase().includes(term));
}

/**
 * How a session is read: its fields one at a time for a pattern, its whole text for words.
 *
 * The difference is what an anchor means. `^postgres` is a claim about the *user*, so a pattern has
 * to meet each field on its own; a word search is a claim about the machine, and somebody typing
 * `postgres@db.internal:5432` means the address rather than three fields at once.
 */
function sessionMatcher(matches: (subject: string) => boolean, regex: boolean): (session: Session) => boolean {
  return regex ? (session) => parts(session).some(matches) : (session) => matches(haystack(session));
}

/** The compiled pattern, or nothing when it is not one. */
function compileRegex(query: string): RegExp | null {
  try {
    return new RegExp(query, "i");
  } catch {
    return null;
  }
}

/** One term per word, lowercased, blanks dropped. */
function terms(query: string): readonly string[] {
  return query
    .trim()
    .toLowerCase()
    .split(/\s+/)
    .filter((term) => term !== "");
}

/** Everything about a session somebody might type to find it, as one line of text. */
function haystack(session: Session): string {
  return parts(session).join(" ");
}

/** The same thing as the separate fields a pattern is matched against, address and all. */
function parts(session: Session): readonly string[] {
  return [
    session.name,
    session.host,
    session.user,
    `${session.user}@${session.host}:${session.port}`,
    String(session.port),
    session.protocol,
  ];
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
