/**
 * What a search keeps: which sessions, which folders, and which folders have to be drawn open.
 *
 * The rules under test are the two at the top of the module — a folder is kept for what is inside
 * it (or for its own name), and everything that is kept is kept open — plus the one thing a search
 * must never do: lose the folder a match is in, which would read as "nothing was found".
 */
import { describe, expect, it } from "vitest";

import {
  NO_FILTER,
  filterIsOn,
  filterLibrary,
  matchesQuery,
  passesMethods,
  regexIsInvalid,
  type SessionFilter,
} from "./filter";
import type { AuthMethod } from "@/types/generated/protocols";
import type { Session, SessionGroup } from "@/types/session";

function session(id: string, name: string, overrides: Partial<Session> = {}): Session {
  return {
    id,
    name,
    protocol: "ssh",
    host: "example.test",
    port: 22,
    user: "root",
    auth: { method: "password", hasSecret: false },
    terminal: { terminalType: "xterm-256color", keepaliveSeconds: 30, forwardAgent: false, record: false },
    updatedAt: 1_700_000_000,
    ...overrides,
  };
}

const work: SessionGroup = { id: "group-1", name: "Work", order: 0, pinned: false };
const lab: SessionGroup = { id: "group-2", name: "Lab", order: 1, pinned: false };
const rack: SessionGroup = { id: "group-3", name: "Rack", order: 0, parentId: work.id, pinned: false };

const web = session("session-1", "web", { host: "web.example.test", user: "deploy" });
const db = session("session-2", "db", {
  groupId: work.id,
  host: "db.internal",
  port: 5432,
  user: "postgres",
  auth: { method: "publicKey", hasSecret: false },
});
const cache = session("session-3", "cache", { groupId: rack.id, host: "cache.internal" });

const all = [work, lab, rack];
const everything = [web, db, cache];

function query(text: string): SessionFilter {
  return { query: text, methods: new Set(), regex: false, onlyOpen: false };
}

function methods(...list: AuthMethod[]): SessionFilter {
  return { query: "", methods: new Set(list), regex: false, onlyOpen: false };
}

function pattern(text: string): SessionFilter {
  return { query: text, methods: new Set(), regex: true, onlyOpen: false };
}

function openOnly(): SessionFilter {
  return { query: "", methods: new Set(), regex: false, onlyOpen: true };
}

/** The names of what is left, groups and sessions in one list, which is what a reader sees. */
function kept(filter: SessionFilter): string[] {
  const result = filterLibrary(all, everything, filter);
  const groups = new Set(result.groups.map((group) => group.name));
  return [...groups, ...result.sessions.map((session) => session.name)];
}

describe("matching a session by what identifies it", () => {
  it("finds a session by name, host, user or port", () => {
    expect(matchesQuery(web, "web")).toBe(true);
    expect(matchesQuery(web, "web.example")).toBe(true);
    expect(matchesQuery(web, "deploy")).toBe(true);
    expect(matchesQuery(db, "5432")).toBe(true);
  });

  it("finds the address typed the way the row prints it", () => {
    expect(matchesQuery(db, "postgres@db.internal:5432")).toBe(true);
  });

  it("needs every term, so two words narrow rather than widen", () => {
    expect(matchesQuery(db, "postgres db")).toBe(true);
    expect(matchesQuery(db, "postgres web")).toBe(false);
  });

  it("ignores case, which is what a search box is expected to do", () => {
    expect(matchesQuery(web, "WEB")).toBe(true);
  });

  it("keeps everything when there is no query at all", () => {
    expect(matchesQuery(web, "")).toBe(true);
    expect(matchesQuery(web, "   ")).toBe(true);
  });

  it("keeps only the methods that were asked for, and everything when none were", () => {
    expect(passesMethods(db, new Set<AuthMethod>())).toBe(true);
    expect(passesMethods(db, new Set<AuthMethod>(["publicKey"]))).toBe(true);
    expect(passesMethods(db, new Set<AuthMethod>(["password"]))).toBe(false);
  });
});

describe("reading the query as a regular expression", () => {
  it("matches each field on its own, so an anchor is about one of them", () => {
    // Anchors are the point: a term search cannot say "starts with this", and `^postgres` is a
    // claim about the *user* rather than about the address and the protocol strung together.
    expect(matchesQuery(db, "^postgres@db", true)).toBe(true);
    expect(matchesQuery(db, "postgres|nothing", true)).toBe(true);
    expect(matchesQuery(web, "^root$", true)).toBe(false);
  });

  it("ignores case, like the word search does, because a pattern cannot say so itself", () => {
    expect(matchesQuery(web, "WEB", true)).toBe(true);
  });

  it("says when a pattern does not compile, and matches nothing with it", () => {
    // A search that is silently ignored is worse than one that is visibly broken: the box draws
    // its own state from this, and the list goes empty rather than showing everything again.
    expect(regexIsInvalid("([")).toBe(true);
    expect(regexIsInvalid("^postgres")).toBe(false);
    expect(matchesQuery(web, "([", true)).toBe(false);
  });

  it("lights up a folder whose name matches the pattern", () => {
    expect(kept(pattern("^work$"))).toEqual(["Work", "Rack", "db", "cache"]);
  });
});

describe("showing only what is open", () => {
  it("keeps the sessions something is open for, and the folders they are in", () => {
    const result = filterLibrary(all, everything, openOnly(), new Set(["session-3"]));

    expect(result.sessions.map((session) => session.name)).toEqual(["cache"]);
    expect(result.groups.map((group) => group.name)).toEqual(["Work", "Rack"]);
  });

  it("keeps nothing when nothing is open", () => {
    expect(kept(openOnly())).toEqual([]);
  });

  it("counts as narrowing, on its own", () => {
    expect(filterIsOn(openOnly())).toBe(true);
  });
});

describe("what the list shows while it is narrowed", () => {
  it("keeps the folder a match is in, and everything above it", () => {
    const result = filterLibrary(all, everything, query("cache"));

    expect(result.sessions.map((session) => session.name)).toEqual(["cache"]);
    expect(result.groups.map((group) => group.name)).toEqual(["Work", "Rack"]);
    expect([...result.revealed].sort()).toEqual(["group-1", "group-3"]);
  });

  it("keeps a folder open when what is inside it matched, whatever its own state was", () => {
    // Which is the whole point: the search asked for it, and a match somebody cannot see is a
    // search that answered "nothing found".
    const result = filterLibrary(all, everything, query("db"));

    expect(result.revealed.has(work.id)).toBe(true);
  });

  it("drops a folder that holds nothing matching", () => {
    expect(kept(query("web"))).toEqual(["web"]);
  });

  it("keeps a folder whose own name matches, with everything inside it", () => {
    // The whole subtree, not just the folder's own sessions: somebody who typed a folder's name is
    // asking to see the folder, and a folder drawn half-open is a folder they would have to open
    // one group at a time.
    expect(kept(query("work"))).toEqual(["Work", "Rack", "db", "cache"]);
    expect(kept(query("rack"))).toEqual(["Work", "Rack", "cache"]);
  });

  it("keeps a folder that matched by name even when a filter emptied it out", () => {
    // The folder is what was asked for, so it is drawn saying it has nothing in it — which is a
    // true answer, unlike a folder that vanishes.
    const filter: SessionFilter = {
      query: "work",
      methods: new Set<AuthMethod>(["agent"]),
      regex: false,
      onlyOpen: false,
    };

    expect(kept(filter)).toEqual(["Work"]);
  });

  it("narrows by method on its own, with no search at all", () => {
    expect(kept(methods("publicKey"))).toEqual(["Work", "db"]);
  });

  it("keeps a folder for what is inside it, and drops one that holds nothing that passed", () => {
    // An empty query is a substring of every name, so this is the case where "a folder whose name
    // matches" has to mean nothing: only the folders on the way to a session that passed are drawn,
    // and the empty one is not — a heading with nothing under it is noise while somebody is hunting.
    const names = kept(methods("password"));

    expect(names).toEqual(["Work", "Rack", "web", "cache"]);
    expect(names).not.toContain("Lab");
  });

  it("keeps nothing at all when no session passes", () => {
    expect(kept(methods("agent"))).toEqual([]);
  });

  it("keeps the whole library when nothing is asked for", () => {
    const result = filterLibrary(all, everything, NO_FILTER);

    expect(result.sessions).toBe(everything);
    expect(result.groups).toBe(all);
    expect(result.revealed.size).toBe(0);
  });

  it("knows when it is narrowing and when it is not", () => {
    expect(filterIsOn(NO_FILTER)).toBe(false);
    expect(filterIsOn(query("   "))).toBe(false);
    expect(filterIsOn(query("web"))).toBe(true);
    expect(filterIsOn(methods("password"))).toBe(true);
  });
});
