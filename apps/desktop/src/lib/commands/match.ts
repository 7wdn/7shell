/**
 * How well a candidate answers what was typed, and which characters answered it.
 *
 * The tiers are ordered and kept apart by their numbers, so an exact hit beats a prefix, a prefix
 * beats matching the initials of a candidate's words (`gc` → `git commit`), a run of characters beats
 * letters scattered through the candidate, and a shorter candidate wins inside a tier — between `ls`
 * and `lsof`, what somebody typed is the one they meant.
 *
 * The ranges come back with the score because the menu highlights them, and a second function that
 * decided for itself which characters matched would be a second answer to the same question.
 */

export interface MatchRange {
  readonly start: number;
  readonly end: number;
}

/** What a candidate that does not answer the prefix at all scores. */
export const NO_MATCH = -1;

export interface TextMatch {
  readonly score: number;
  readonly ranges: readonly MatchRange[];
}

interface Match {
  readonly score: number;
  readonly ranges: readonly MatchRange[];
}

/** A character that starts a word inside a candidate: after a space, a dash, an underscore, a slash. */
function isBoundary(text: string, index: number): boolean {
  if (index === 0) {
    return true;
  }
  return /[\s\-_/.]/.test(text[index - 1] ?? "");
}

function boundaryBonus(text: string, index: number): number {
  return isBoundary(text, index) ? 400 : Math.max(0, 180 - index * 12);
}

/** Where each word starts: index 0, and every position after a boundary character. */
function wordStarts(text: string): number[] {
  const starts: number[] = [];
  for (let index = 0; index < text.length; index += 1) {
    if (isBoundary(text, index)) {
      starts.push(index);
    }
  }
  return starts;
}

interface Initials {
  readonly every: boolean;
  readonly ranges: readonly MatchRange[];
}

/** The first letters of the words, for `gc` answering `git commit`. */
function initialsMatch(text: string, needle: string): Initials | null {
  const starts = wordStarts(text);
  if (starts.length < needle.length) {
    return null;
  }

  const letters = starts.map((start) => (text[start] ?? "").toLowerCase());
  if (letters.slice(0, needle.length).join("") !== needle) {
    return null;
  }

  return {
    every: starts.length === needle.length,
    ranges: starts.slice(0, needle.length).map((start) => ({ start, end: start + 1 })),
  };
}

interface Scattered {
  readonly first: number;
  readonly gaps: number;
  readonly ranges: readonly MatchRange[];
}

/** Every character of the needle, in order, anywhere in the candidate. */
function scatteredMatch(haystack: string, needle: string): Scattered | null {
  const ranges: MatchRange[] = [];
  let cursor = 0;
  for (const character of needle) {
    const at = haystack.indexOf(character, cursor);
    if (at < 0) {
      return null;
    }
    ranges.push({ start: at, end: at + 1 });
    cursor = at + 1;
  }

  const first = ranges[0]?.start ?? 0;
  const last = ranges.at(-1);
  const span = last === undefined ? 0 : last.end - first;
  return { first, gaps: Math.max(0, span - needle.length), ranges };
}

function matchOf(text: string, prefix: string): Match | null {
  if (prefix === "") {
    return { score: 0, ranges: [] };
  }

  const haystack = text.toLowerCase();
  const needle = prefix.toLowerCase();
  const length = text.length;
  const run = haystack.indexOf(needle);

  if (text === prefix) {
    return { score: 3000 - length, ranges: [{ start: 0, end: prefix.length }] };
  }
  if (haystack === needle) {
    return { score: 2900 - length, ranges: [{ start: 0, end: prefix.length }] };
  }
  if (run === 0) {
    return { score: 2000 - length, ranges: [{ start: 0, end: prefix.length }] };
  }

  const initials = initialsMatch(text, needle);
  if (initials !== null) {
    return { score: (initials.every ? 2400 : 2200) - length, ranges: initials.ranges };
  }

  if (run > 0) {
    return { score: 900 + boundaryBonus(text, run) - length, ranges: [{ start: run, end: run + prefix.length }] };
  }

  const scattered = scatteredMatch(haystack, needle);
  if (scattered !== null) {
    const early = scattered.first === 0 ? 120 : Math.max(0, 120 - scattered.first * 15);
    return { score: 700 + early - scattered.gaps * 10 - length, ranges: scattered.ranges };
  }

  return null;
}

/**
 * What `text` answers to `prefix`: the score and the characters that answered.
 *
 * One call rather than two, because the ranges are only consistent with the score that was chosen —
 * a caller asking again would be asking a question that has one answer and could get another.
 */
export function matchText(text: string, prefix: string): TextMatch {
  return matchOf(text, prefix) ?? { score: NO_MATCH, ranges: [] };
}
