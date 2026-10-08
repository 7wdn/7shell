/**
 * Where a path somebody is typing points, and what an entry in it would complete to.
 *
 * Paths are the one thing on this side that can only be answered by asking the host: the listing is
 * read over the SFTP channel the terminal's own file panel has open, and the reading is the caller's
 * business. What lives here is the string work — which directory a token means (absolute, relative,
 * or bare), and what the token becomes when somebody takes an entry.
 *
 * The token is kept whole in the answer on purpose: what a candidate replaces is the token with its
 * last segment rewritten, so `/va` becomes `/var/`, `../lo` becomes `../logs/`, and a bare name stays
 * bare. Nothing here joins a path the host sent back: entries carry their own full path, and only the
 * text being typed is ever worked out from text.
 */
import { matchText, NO_MATCH } from "@/lib/commands/match";
import type { SftpEntry } from "@/types/session";

/** How many entries of a directory are worth offering: a listing is read, not scrolled. */
export const MAX_PATH_CANDIDATES = 60;

/** Commands whose first word is usually a path, so a bare name may be one too. */
const PATH_COMMANDS = new Set([
  "cd",
  "cat",
  "chmod",
  "chown",
  "cp",
  "df",
  "du",
  "find",
  "grep",
  "gzip",
  "head",
  "less",
  "ln",
  "ls",
  "mkdir",
  "more",
  "mv",
  "nano",
  "rm",
  "rmdir",
  "stat",
  "tail",
  "tar",
  "touch",
  "unzip",
  "vi",
  "vim",
  "zip",
]);

/** Whether a token can only be a path, whatever command it follows: `/etc`, `./run.sh`, `../logs`. */
export function looksLikePath(token: string): boolean {
  return token.startsWith("/") || token.startsWith("./") || token.startsWith("../");
}

/** Whether a command's arguments are worth reading a directory for. */
export function takesPaths(command: string): boolean {
  return PATH_COMMANDS.has(command.toLowerCase());
}

export interface PathQuery {
  /** The directory to read. */
  readonly directory: string;
  /** What has been typed of the last segment, which is what the listing is narrowed by. */
  readonly name: string;
  /** The token as typed, which is what a candidate rewrites. */
  readonly token: string;
}

/** A path, with its `.` and `..` segments settled and its trailing slash gone. */
function normalize(parts: readonly string[]): string {
  const settled: string[] = [];
  for (const part of parts) {
    if (part === "" || part === ".") {
      continue;
    }
    if (part === "..") {
      settled.pop();
      continue;
    }
    settled.push(part);
  }
  return `/${settled.join("/")}`;
}

/**
 * The query a token stands for.
 *
 * `null` when there is nothing to resolve against: a bare name or a relative path with no known
 * directory is not a question this side can ask, while an absolute one needs no starting point and is
 * answered anyway.
 */
export function pathQueryAt(token: string, cwd: string | null): PathQuery | null {
  const absolute = token.startsWith("/");
  if (!absolute && cwd === null) {
    return null;
  }

  const parts = token.split("/");
  const name = parts.pop() ?? "";
  const base = (absolute ? "/" : (cwd ?? "/")).split("/");
  return { directory: normalize([...base, ...parts]), name, token };
}

/** What a row reads as: a directory says so with the slash a shell would have put there. */
export function entryLabel(entry: SftpEntry): string {
  return entry.kind === "directory" ? `${entry.name}/` : entry.name;
}

/** What taking an entry leaves in place of the token: the token's directory part, plus the row. */
export function completionTextFor(query: PathQuery, label: string): string {
  const stem = query.name === "" ? query.token : query.token.slice(0, query.token.length - query.name.length);
  return `${stem}${label}`;
}

export interface EntryMatch {
  readonly entry: SftpEntry;
  readonly label: string;
  readonly insert: string;
  readonly score: number;
  readonly ranges: ReturnType<typeof matchText>["ranges"];
}

/**
 * The entries that answer what was typed, in the order the host listed them.
 *
 * Cut at [`MAX_PATH_CANDIDATES`]: a directory with five thousand entries is not a list anybody reads
 * while typing, and the ones that answer come first because the filter runs before the cut.
 */
export function entryMatches(entries: readonly SftpEntry[], query: PathQuery): readonly EntryMatch[] {
  const matches: EntryMatch[] = [];
  for (const entry of entries) {
    const match = matchText(entry.name, query.name);
    if (match.score === NO_MATCH) {
      continue;
    }
    const label = entryLabel(entry);
    matches.push({
      entry,
      label,
      insert: completionTextFor(query, label),
      score: match.score,
      ranges: match.ranges,
    });
    if (matches.length >= MAX_PATH_CANDIDATES) {
      break;
    }
  }
  return matches;
}
