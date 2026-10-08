/**
 * Building the list that goes under the input bar.
 *
 * Where two candidates answer the prefix equally well, the one the person made comes first, then the
 * one they have run, then the one we ship — but match quality decides before any of that, and an
 * exact hit decides outright, so completing `git` lands on `git` rather than on a saved line that
 * happens to begin with the same three letters.
 *
 * The command word and the first argument are completed; what comes after that is left alone. A
 * command word is answered from what this application knows — commands, saved lines, history — and
 * the first argument from two sources that are about that command: the table of words it is usually
 * followed by, and what somebody typed there before. Deeper than that there is no source of truth
 * here, and guessing would put a wrong word into a line somebody is about to run.
 */
import { matchText, NO_MATCH, type MatchRange } from "@/lib/commands/match";
import { entryMatches, looksLikePath, pathQueryAt, takesPaths } from "@/lib/commands/paths";
import { fillSlots, slotsIn } from "@/lib/commands/slots";
import { completionSiteAt, type CompletionSite } from "@/lib/commands/tokenize";
import type { CatalogEntry, CatalogKind } from "@/lib/commands/catalog";
import type { CommandHistoryEntry, Snippet } from "@/types/snippet";
import type { SftpEntry } from "@/types/session";

export type CompletionKind = "command" | "builtin" | "subcommand" | "saved" | "history" | "path";

export interface CompletionCandidate {
  readonly label: string;
  readonly detail: string | null;
  /** What replaces `[from, caret)` when somebody takes this candidate. */
  readonly insert: string;
  readonly kind: CompletionKind;
  readonly from: number;
  readonly score: number;
  /** True when the candidate is a whole line rather than a word: taking it ends the line's sentence. */
  readonly wholeLine: boolean;
  /**
   * Whether a space belongs after it.
   *
   * A command word is always followed by one; a file lands at the end of an argument; a directory is
   * half a path, and taking it leaves the caret where the next keystroke continues the path.
   */
  readonly trailingSpace: boolean;
  /** The characters of the label the prefix answered, for the highlight. */
  readonly labelRanges: readonly MatchRange[];
  /** The same for the detail, which is where a saved command's own words match. */
  readonly detailRanges: readonly MatchRange[];
}

export interface CompletionSources {
  readonly saved: readonly Snippet[];
  readonly history: readonly CommandHistoryEntry[];
  readonly catalog: readonly CatalogEntry[];
}

/**
 * What is in a directory on the far side.
 *
 * Reading is the caller's business — this side has no socket — so the entries arrive already asked
 * for, with `null` meaning "nothing has been read yet, ask again in a moment". A path is the one
 * source that cannot be answered from memory, and the one whose answer may arrive after the keystroke
 * that asked for it.
 */
export interface PathSource {
  /** The directory a relative path is resolved against, or `null` when nothing is known. */
  readonly cwd: string | null;
  readonly entries: (directory: string) => readonly SftpEntry[] | null;
}

export interface CompletionRequest {
  readonly line: string;
  readonly caret: number;
  readonly sources: CompletionSources;
  /** The one line about a catalog command, or `null` when this build has none for it. */
  readonly describe: (name: string) => string | null;
  /** What a command is usually followed by, in the order worth reading. */
  readonly subcommands: (command: string) => readonly string[];
  /** The one line about one of those words, or `null` when this build has none for it. */
  readonly describeSubcommand: (command: string, word: string) => string | null;
  /** The word for a kind, shown where a candidate has nothing better to say. */
  readonly kindLabel: (kind: CatalogKind | "subcommand" | "path") => string;
  /** Where an argument that is a path is read from. Left out when there is no host to ask. */
  readonly paths?: PathSource;
  /**
   * What taking a row before is worth, for a row somebody has taken before.
   *
   * A hint, not a judgement: the value is small enough that a better match still wins, and the one
   * place it comes from is the caller — the engine keeps nothing between calls.
   */
  readonly acceptance?: (insert: string) => number;
  readonly limit?: number;
}

/** What a candidate being there is worth before matching: the order the sources are trusted in. */
const SOURCE_BOOST: Readonly<Record<CompletionKind, number>> = {
  saved: 400,
  subcommand: 400,
  // Under the table and over the history: what the host actually has in a directory is a better guess
  // than what somebody typed once, and a worse one than the words the command is usually given.
  path: 380,
  history: 300,
  command: 100,
  builtin: 60,
};

const EXACT_BOOST = 10_000;
const DEFAULT_LIMIT = 200;

/** How much history an empty word offers: the recent part of it, not the whole thing. */
const RECENT_HISTORY = 20;

/** The values a saved line was last sent with, by slot number, for filling it in on the way in. */
function lastUsedValues(snippet: Snippet): string[] {
  const values: string[] = [];
  for (const slot of slotsIn(snippet.command)) {
    values[slot - 1] = snippet.slotHistory[slot - 1]?.[0] ?? "";
  }
  return values;
}

/**
 * A line with one word in it is a word.
 *
 * `uptime` taken out of the library leaves a space after it and opens what can follow, exactly as
 * `uptime` taken out of the catalogue does; `cd /srv/app` does neither, because a line is not half
 * of anything. The rule is about what is being inserted rather than about where it came from, and
 * the two sources meet in the same place: three letters should leave the line ready for the next
 * word, and a whole line should be left alone.
 */
function oneWordOnly(line: string): boolean {
  return !/\s/.test(line);
}

function fromSaved(saved: readonly Snippet[], site: CompletionSite): CompletionCandidate[] {
  const candidates: CompletionCandidate[] = [];
  for (const [index, snippet] of saved.entries()) {
    // Switched off is out of the list and still in the library: what somebody stopped using is not
    // the same as what they deleted.
    if (!snippet.enabled) {
      continue;
    }

    // Three things answer for a saved command: the trigger somebody typed to reach it, its name, and
    // the line itself. The best of them decides the row.
    const byTrigger = matchText(snippet.prefix, site.prefix);
    const byName = matchText(snippet.name, site.prefix);
    const byCommand = matchText(snippet.command, site.prefix);
    const best = [byTrigger, byName, byCommand].reduce((left, right) => (right.score > left.score ? right : left));
    if (best.score === NO_MATCH) {
      continue;
    }

    const insert = fillSlots(snippet.command, lastUsedValues(snippet));
    // A whole line that is already in the line is not a completion: taking it would change nothing,
    // and a row that does nothing is a row in the way of the one that does. Whole lines are what the
    // history list is for; the menu is for finishing a word.
    if (insert === site.prefix) {
      continue;
    }

    // A trigger typed out in full is the whole question: it is short on purpose, and somebody who
    // typed all of it means this command and nothing that merely starts the same way.
    const triggered =
      snippet.prefix !== "" && snippet.prefix.toLowerCase() === site.prefix.toLowerCase() ? EXACT_BOOST : 0;

    // The row shows the trigger it was reached by, and the line behind it: a match nobody can see is
    // a match nobody trusts. The command's own highlight is moved along by what was put in front of it.
    const offset = snippet.prefix === "" ? 0 : snippet.prefix.length + " · ".length;
    const detail = snippet.prefix === "" ? snippet.command : `${snippet.prefix} · ${snippet.command}`;
    const detailRanges =
      byTrigger.score > byCommand.score
        ? byTrigger.ranges
        : byCommand.ranges.map((range) => ({ start: range.start + offset, end: range.end + offset }));

    candidates.push({
      label: snippet.name,
      detail,
      insert,
      kind: "saved",
      from: site.start,
      score: best.score + SOURCE_BOOST.saved - index + triggered,
      wholeLine: !oneWordOnly(insert),
      trailingSpace: oneWordOnly(insert),
      labelRanges: byName.ranges,
      detailRanges,
    });
  }
  return candidates;
}

function fromHistory(history: readonly CommandHistoryEntry[], site: CompletionSite): CompletionCandidate[] {
  const candidates: CompletionCandidate[] = [];
  for (const [index, entry] of history.entries()) {
    const match = matchText(entry.command, site.prefix);
    if (match.score === NO_MATCH || entry.command === site.prefix) {
      continue;
    }

    candidates.push({
      label: entry.command,
      detail: null,
      insert: entry.command,
      kind: "history",
      from: site.start,
      // Newer first: the history arrives newest first, and what somebody ran a minute ago is a better
      // guess than what they ran last week.
      score: match.score + SOURCE_BOOST.history - index,
      // A line somebody ran is taken as it stands, unless it is one word: `uptime` from last week is
      // the same word the catalogue offers, and it should behave the same way.
      wholeLine: !oneWordOnly(entry.command),
      trailingSpace: oneWordOnly(entry.command),
      labelRanges: match.ranges,
      detailRanges: [],
    });
  }
  return candidates;
}

function fromCatalog(
  catalog: readonly CatalogEntry[],
  site: CompletionSite,
  describe: (name: string) => string | null,
  kindLabel: (kind: CatalogKind) => string,
): CompletionCandidate[] {
  const candidates: CompletionCandidate[] = [];
  for (const entry of catalog) {
    const match = matchText(entry.name, site.prefix);
    if (match.score === NO_MATCH) {
      continue;
    }

    const kind: CompletionKind = entry.kind === "builtin" ? "builtin" : "command";
    candidates.push({
      label: entry.name,
      detail: describe(entry.name) ?? kindLabel(entry.kind),
      insert: entry.name,
      kind,
      from: site.start,
      score: match.score + SOURCE_BOOST[kind],
      wholeLine: false,
      trailingSpace: true,
      labelRanges: match.ranges,
      detailRanges: [],
    });
  }
  return candidates;
}

/** The same line once: an entry that is saved and was also run is one row, from the better source. */
function dedupe(candidates: readonly CompletionCandidate[]): CompletionCandidate[] {
  const byInsert = new Map<string, CompletionCandidate>();
  for (const candidate of candidates) {
    const kept = byInsert.get(candidate.insert);
    if (kept === undefined || candidate.score > kept.score) {
      byInsert.set(candidate.insert, candidate);
    }
  }
  return [...byInsert.values()];
}

function compare(left: CompletionCandidate, right: CompletionCandidate): number {
  if (left.score !== right.score) {
    return right.score - left.score;
  }
  if (left.label !== right.label) {
    return left.label < right.label ? -1 : 1;
  }
  return left.kind === right.kind ? 0 : left.kind < right.kind ? -1 : 1;
}

/**
 * The words a command is usually followed by.
 *
 * The table's order is deliberate, so where two words answer the prefix equally well the one written
 * first wins: `status` is more often what somebody wants than `show`.
 */
function fromSubcommands(
  command: string,
  site: CompletionSite,
  words: readonly string[],
  describe: (command: string, word: string) => string | null,
  kindLabel: (kind: CatalogKind | "subcommand") => string,
): CompletionCandidate[] {
  const candidates: CompletionCandidate[] = [];
  for (const [index, word] of words.entries()) {
    const match = matchText(word, site.prefix);
    if (match.score === NO_MATCH) {
      continue;
    }

    candidates.push({
      label: word,
      detail: describe(command, word) ?? kindLabel("subcommand"),
      insert: word,
      kind: "subcommand",
      from: site.start,
      score: match.score + SOURCE_BOOST.subcommand - index,
      wholeLine: false,
      trailingSpace: true,
      labelRanges: match.ranges,
      detailRanges: [],
    });
  }
  return candidates;
}

/**
 * What somebody typed in this place before, after this command.
 *
 * The word at the same position in a line that starts with the same command: `systemctl` and then
 * `restart` has been run here before, and on a host whose units are somebody's own that beats a table.
 * Lines that start with a wrapper (`sudo systemctl …`) are skipped rather than re-aligned — working out
 * where the command sits in a line somebody else's shell wrote is how a suggestion becomes a wrong
 * word.
 */
function fromHistoryArguments(
  command: string,
  site: CompletionSite,
  history: readonly CommandHistoryEntry[],
): CompletionCandidate[] {
  const candidates: CompletionCandidate[] = [];
  const seen = new Set<string>();
  for (const [index, entry] of history.entries()) {
    const words = entry.command.trim().split(/\s+/);
    if (words[0]?.toLowerCase() !== command) {
      continue;
    }

    const word = words[site.index];
    if (word === undefined || word === "" || word === site.prefix || seen.has(word)) {
      continue;
    }

    const match = matchText(word, site.prefix);
    if (match.score === NO_MATCH) {
      continue;
    }

    seen.add(word);
    candidates.push({
      label: word,
      detail: null,
      insert: word,
      kind: "history",
      from: site.start,
      score: match.score + SOURCE_BOOST.history - index,
      wholeLine: false,
      trailingSpace: true,
      labelRanges: match.ranges,
      detailRanges: [],
    });
  }
  return candidates;
}

export function buildCommandCompletions(request: CompletionRequest): readonly CompletionCandidate[] {
  const site = completionSiteAt(request.line, request.caret);
  if (site.comment || site.quoted) {
    return [];
  }

  const candidates = site.commandPosition ? atCommandWord(request, site) : atArgument(request, site);
  const boost = request.acceptance ?? (() => 0);
  const boosted = candidates.map((candidate) => {
    const remembered = boost(candidate.insert);
    const score = candidate.score + remembered;
    return candidate.label === site.prefix || candidate.insert === site.prefix
      ? { ...candidate, score: score + EXACT_BOOST }
      : { ...candidate, score };
  });

  return dedupe(boosted)
    .sort(compare)
    .slice(0, request.limit ?? DEFAULT_LIMIT);
}

/** The word itself: what this application knows as a command. */
function atCommandWord(request: CompletionRequest, site: CompletionSite): CompletionCandidate[] {
  const listing = site.prefix === "";
  return [
    ...fromSaved(request.sources.saved, site),
    ...fromHistory(listing ? request.sources.history.slice(0, RECENT_HISTORY) : request.sources.history, site),
    // What we ship is offered while somebody is typing a word, not as a list to browse: an empty
    // word is a question about their own lines, not an invitation to read a catalogue.
    ...(listing ? [] : fromCatalog(request.sources.catalog, site, request.describe, request.kindLabel)),
  ];
}

/**
 * What the first word after the command can be.
 *
 * The table says what a command is followed by, not what its second argument is, so it is only asked
 * at the first position — while the history and the host's own directories are worth asking at any of
 * them: `cp a b /tm` is the fourth word, and `/tm` is still a path.
 */
function atArgument(request: CompletionRequest, site: CompletionSite): CompletionCandidate[] {
  const command = site.command;
  if (command === null) {
    return [];
  }

  const words = site.index === 1 ? request.subcommands(command) : [];
  const paths =
    request.paths !== undefined && (looksLikePath(site.prefix) || takesPaths(command))
      ? fromPaths(request.paths, site, request.kindLabel)
      : [];

  return [
    ...fromSubcommands(command, site, words, request.describeSubcommand, request.kindLabel),
    ...fromHistoryArguments(command, site, request.sources.history),
    ...paths,
  ];
}

/**
 * Entries in the directory the token points at, as far as this side has read it.
 *
 * `null` from the source means "not read yet" rather than "empty": there is nothing to offer this
 * keystroke, and the caller's read will bring the list in a moment.
 */
function fromPaths(
  source: PathSource,
  site: CompletionSite,
  kindLabel: (kind: CatalogKind | "subcommand" | "path") => string,
): CompletionCandidate[] {
  const query = pathQueryAt(site.prefix, source.cwd);
  if (query === null) {
    return [];
  }

  const entries = source.entries(query.directory);
  if (entries === null) {
    return [];
  }

  return entryMatches(entries, query).map((match) => ({
    label: match.label,
    detail: kindLabel("path"),
    insert: match.insert,
    kind: "path" as const,
    from: site.start,
    score: match.score + SOURCE_BOOST.path,
    wholeLine: false,
    // A directory is half a path: the next keystroke continues it, and a space would be in the way.
    trailingSpace: match.entry.kind !== "directory",
    labelRanges: match.ranges,
    detailRanges: [],
  }));
}
