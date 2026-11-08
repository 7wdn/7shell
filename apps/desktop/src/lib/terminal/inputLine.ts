/**
 * The line somebody is typing into a terminal, as far as the keystrokes say.
 *
 * There is no line editor on this side to ask: the shell's own is on the other end of a pty, and the
 * only thing this side sees is the bytes that were sent. So this models the small part completion
 * needs — printable characters, the ways to erase them, and Enter — and refuses to guess at the
 * rest. A completion, a history recall, a pasted script: each changes the line without saying what
 * it became, and a line half-known is worse than a line unknown, because a candidate built from it
 * would be inserted into text nobody can see.
 *
 * Once a line is untrusted it stays untrusted until Enter, Ctrl+U or Ctrl+C: those are the moments
 * the shell itself starts a line this side can follow again.
 */

/** Bracketed paste, which xterm puts around a paste so a shell knows not to run it line by line. */
const PASTE_START = "\x1b[200~";
const PASTE_END = "\x1b[201~";

export interface InputLineState {
  /** What has been typed on this line, from the first character. */
  readonly line: string;
  /** Whether the typed line is still known well enough to complete from. */
  readonly trusted: boolean;
  /** Whether the cursor is at the end of the line. */
  readonly atEnd: boolean;
  /** Enter was pressed: the line is gone and what runs next is the shell's business. */
  readonly executed: boolean;
}

export class InputLine {
  private line = "";
  private trusted = true;
  private atEnd = true;
  private pasting = false;

  reset(): void {
    this.line = "";
    this.trusted = true;
    this.atEnd = true;
    this.pasting = false;
  }

  /** After a completion wrote a new line: that line is known exactly, and the cursor is at its end. */
  setLine(line: string): void {
    this.line = line;
    this.trusted = true;
    this.atEnd = true;
  }

  snapshot(): InputLineState {
    return { line: this.line, trusted: this.trusted, atEnd: this.atEnd, executed: false };
  }

  /** Feeds what was typed since the last call and returns what the line became. */
  feed(data: string): InputLineState {
    let executed = false;

    for (let index = 0; index < data.length; index += 1) {
      const character = data[index] ?? "";

      if (this.pasting) {
        if (data.startsWith(PASTE_END, index)) {
          this.pasting = false;
          index += PASTE_END.length - 1;
        }
        continue;
      }

      if (character === "\x1b") {
        if (data.startsWith(PASTE_START, index)) {
          this.pasting = true;
          index += PASTE_START.length - 1;
          // What a paste becomes is the shell's reading of it, not this side's.
          this.trusted = false;
          continue;
        }
        this.untrusted();
        index = skipEscape(data, index);
        continue;
      }

      if (character === "\r" || character === "\n") {
        executed = true;
        this.reset();
        continue;
      }

      if (character === "\x7f" || character === "\b") {
        if (this.trusted) {
          this.line = removeLastCodePoint(this.line);
        }
        this.atEnd = true;
        continue;
      }

      if (character === "\x15" || character === "\x03") {
        // Ctrl-U kills the line and Ctrl-C abandons it; both leave a line this side can follow.
        this.line = "";
        this.trusted = true;
        this.atEnd = true;
        continue;
      }

      if (character === "\x17") {
        if (this.trusted) {
          this.line = removeLastWord(this.line);
        }
        this.atEnd = true;
        continue;
      }

      if (character < " ") {
        // Tab completion and the rest: the shell acts on the line and this side does not see what
        // it became.
        this.untrusted();
        continue;
      }

      if (this.trusted) {
        this.line += character;
      }
      this.atEnd = true;
    }

    return { line: this.line, trusted: this.trusted, atEnd: this.atEnd, executed };
  }

  private untrusted(): void {
    this.trusted = false;
    this.atEnd = false;
  }
}

function removeLastCodePoint(text: string): string {
  const characters = [...text];
  characters.pop();
  return characters.join("");
}

function removeLastWord(text: string): string {
  return text.replace(/\S+\s*$/, "");
}

/** The index of the last byte of the escape sequence that starts at `start`. */
function skipEscape(data: string, start: number): number {
  let index = start + 1;
  if (data[index] === "[") {
    index += 1;
    while (index < data.length && !isFinalByte(data[index] ?? "")) {
      index += 1;
    }
    return index;
  }
  // A sequence that is not CSI is at most one byte beyond its introducer.
  return index;
}

function isFinalByte(character: string): boolean {
  return character >= "@" && character <= "~";
}
