/**
 * Where the caret is in a shell line, in the terms completion needs.
 *
 * A shell line is words separated by whitespace, with `'…'` and `"…"` grouping and `\` escaping the
 * next character, and with `|`, `;`, `&&` and `||` starting a new command. Completion needs very
 * little of that: which word the caret is in, where that word starts — the text an accepted candidate
 * replaces — whether the word is the command word or an argument, and whether the caret is somewhere
 * nothing should be offered at all, inside a comment or inside quotes.
 *
 * The interface owns this line and nothing else: the shell on the far end of the pty has its own
 * editor, and what it would make of a line is not knowable from here. So this reads one line of
 * text, with no claim about what happens to it afterwards.
 */

/** One word as it was found, before anything is read into it. */
interface ScannedWord {
  readonly start: number;
  readonly end: number;
  readonly text: string;
  /** The caret is inside a quote this word opened and never closed. */
  readonly openQuote: boolean;
  readonly separator: boolean;
  readonly comment: boolean;
}

/** Words that stand in front of the command rather than being one. */
const WRAPPERS = new Set([
  "sudo",
  "doas",
  "env",
  "nohup",
  "time",
  "nice",
  "ionice",
  "command",
  "exec",
  "setsid",
  "xargs",
  "stdbuf",
]);

const SEPARATOR_CHARACTERS = new Set(["|", ";", "&"]);

function isAssignment(text: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*=/.test(text);
}

/** Everything before the caret, cut into words, separators and comments. */
function scan(line: string, caret: number): ScannedWord[] {
  const words: ScannedWord[] = [];
  let index = 0;

  while (index < caret) {
    const character = line[index] ?? "";

    if (character === " " || character === "\t") {
      index += 1;
      continue;
    }

    if (character === "#" && (index === 0 || /[\s|;&]/.test(line[index - 1] ?? ""))) {
      words.push({
        start: index,
        end: caret,
        text: line.slice(index, caret),
        openQuote: false,
        separator: false,
        comment: true,
      });
      return words;
    }

    if (SEPARATOR_CHARACTERS.has(character)) {
      const doubled = character !== ";" && line[index + 1] === character;
      const end = Math.min(index + (doubled ? 2 : 1), caret);
      words.push({
        start: index,
        end,
        text: line.slice(index, end),
        openQuote: false,
        separator: true,
        comment: false,
      });
      index = end;
      continue;
    }

    let cursor = index;
    let quote: string | null = null;
    while (cursor < caret) {
      const at = line[cursor] ?? "";
      if (at === "\\") {
        cursor += 2;
        continue;
      }
      if (quote !== null) {
        if (at === quote) {
          quote = null;
        }
        cursor += 1;
        continue;
      }
      if (at === "'" || at === '"') {
        quote = at;
        cursor += 1;
        continue;
      }
      if (at === " " || at === "\t" || SEPARATOR_CHARACTERS.has(at)) {
        break;
      }
      cursor += 1;
    }

    const end = Math.min(cursor, caret);
    words.push({
      start: index,
      end,
      text: line.slice(index, end),
      openQuote: quote !== null,
      separator: false,
      comment: false,
    });
    index = end;
  }

  return words;
}

/**
 * The command a word belongs to, and where it sits among the words before the caret.
 *
 * The wrappers in front of it are skipped: `sudo ls` is a command word, and `sudo -u root ls` is not,
 * because once the wrapper has arguments of its own there is no way to tell from here which word was
 * meant to be the command. `at` is what makes the position after it countable: `sudo systemctl ` is
 * the first word after the command, not the second.
 */
function commandBefore(words: readonly ScannedWord[]): { command: string | null; at: number } {
  for (const [at, word] of words.entries()) {
    const text = word.text;
    if (isAssignment(text) || WRAPPERS.has(text.toLowerCase())) {
      continue;
    }
    return { command: text.toLowerCase(), at };
  }
  return { command: null, at: -1 };
}

export interface CompletionSite {
  /** Where the word starts: what an accepted candidate replaces, up to the caret. */
  readonly start: number;
  readonly prefix: string;
  /** Which word of the command this is: `0` for the command itself, `1` for its first argument. */
  readonly index: number;
  /** The caret is inside quotes, where a shell word is not a command name. */
  readonly quoted: boolean;
  /** The caret is in a comment, where nothing is offered at all. */
  readonly comment: boolean;
  /** The word being typed is the command word, possibly behind `sudo` and friends. */
  readonly commandPosition: boolean;
  /** The command this word is an argument of, or `null` when it is the command word itself. */
  readonly command: string | null;
}

export function completionSiteAt(line: string, caret: number): CompletionSite {
  const words = scan(line, caret);
  const last = words.at(-1);
  const caretWord = last !== undefined && !last.separator && last.end >= caret && last.end > last.start ? last : null;

  const quotedStart = caretWord !== null && (line[caretWord.start] === "'" || line[caretWord.start] === '"');
  const start = caretWord === null ? caret : quotedStart ? caretWord.start + 1 : caretWord.start;
  const prefix = caretWord === null ? "" : line.slice(start, Math.min(caret, caretWord.end));

  const upTo = caretWord === null ? words.length : words.length - 1;
  let segmentStart = 0;
  for (let index = upTo - 1; index >= 0; index -= 1) {
    if (words[index]?.separator === true) {
      segmentStart = index + 1;
      break;
    }
  }

  const before = words.slice(segmentStart, upTo).filter((word) => !word.separator);
  const { command, at } = commandBefore(before);

  return {
    start,
    prefix,
    index: command === null ? 0 : before.length - at,
    quoted: caretWord?.openQuote === true,
    comment: caretWord?.comment === true,
    commandPosition: command === null,
    command,
  };
}
