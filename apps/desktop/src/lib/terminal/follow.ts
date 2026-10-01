/**
 * Where the shell is, as far as this side can tell.
 *
 * A shell does not announce where it is. Nothing in SSH asks for it, an exec channel starts in a
 * directory of its own, and the SFTP session has a third — so there are exactly two ways to know,
 * and this module reads both:
 *
 * 1. **What the shell said.** A shell that reports its own directory (`OSC 7`, which some
 *    distributions set up out of the box) states it exactly, on every prompt.
 * 2. **What the person typed.** `cd /data` names a directory, and a line editor is a small enough
 *    thing to model: keep the current line from the keystrokes and read a `cd` off it at Enter.
 *    It is a reading of what was typed, not a report of what happened, so it refuses anything it
 *    cannot be sure of — a path built from a variable, a glob, a substitution, a `cd` inside a
 *    subshell, a line that history or a completion rewrote underneath it.
 *
 * The second is what makes the mode work on any host; the first is exact wherever it is available.
 * Both end in the same place, and neither is trusted on its own: what a directory reading produces
 * is a directory the browser is asked to list before it goes anywhere.
 *
 * Nothing here touches the terminal. Reading the line is reading what somebody typed into their
 * own shell, and the only thing that leaves this module is the name of a directory.
 */
import { joinPath } from "@/lib/files/format";

/** Bracketed paste, which xterm puts around a paste so a shell knows not to run it line by line. */
const PASTE_START = "\x1b[200~";
const PASTE_END = "\x1b[201~";

/**
 * Shell syntax, in the three kinds of quoting that matter.
 *
 * Bare text can be expanded, globbed, split on whitespace or read as several commands, and none of
 * that can be worked out from here. Inside double quotes only four of those still happen. Inside
 * single quotes nothing does, which is why a path quoted that way is taken exactly as written.
 */
const EXPANDS_BARE = /[$`*?[\]{}<>&|;!()\\]/;
const EXPANDS_IN_DOUBLE = /[$`\\!]/;

/** The directory a completed command line names, or null when it does not name one surely. */
export function directoryInCommand(line: string): string | null {
  const commands = commandsIn(line);
  for (let index = commands.length - 1; index >= 0; index -= 1) {
    const read = readCommand(commands[index] ?? "");
    // The last `cd` in the line is the one the shell ends up running, so a later one that cannot
    // be read is where the reading stops: falling back to an earlier `cd` would move the panel to
    // a directory the shell has already left.
    if (read.kind === "directory") {
      return read.target;
    }
    if (read.kind === "unreadable") {
      return null;
    }
  }
  return null;
}

/** What the shell reported as its own directory, from an `OSC 7` payload: `file://host/path`. */
export function directoryFromOsc7(data: string): string | null {
  if (!data.startsWith("file://")) {
    return null;
  }
  // A report with no path in it at all names no directory; a bare `file://host` would otherwise be
  // read as the root, which is a directory nobody reported being in.
  if (!data.slice("file://".length).includes("/")) {
    return null;
  }
  try {
    const reported = new URL(data);
    if (reported.protocol !== "file:") {
      return null;
    }
    const path = decodeURIComponent(reported.pathname);
    return path === "" ? null : path;
  } catch {
    return null;
  }
}

/**
 * The directory an argument names, given where the shell was and where it started.
 *
 * `shellPath` rather than the directory on screen: the browser can be moved by hand while the shell
 * stays put, and a relative `cd` resolved against a browsed-to directory would land somewhere
 * neither side asked for.
 */
export function resolveDirectory(shellPath: string, home: string, target: string): string | null {
  const text = target.trim();
  if (text === "" || text === "~" || text === "~/") {
    // A `cd` with nowhere to go is the home directory, which is where the browser opened.
    return home;
  }
  if (text.startsWith("~/")) {
    return joinPath(home, text.slice(2));
  }
  if (text.startsWith("~")) {
    // Somebody else's home: the shell can work it out from the password database, and this side
    // has no way to.
    return null;
  }
  if (text === "-") {
    // The shell's own previous directory, which is not the same as the browser's.
    return null;
  }
  return text.startsWith("/") ? text : joinPath(shellPath, text);
}

/**
 * The line somebody is typing, as far as the keystrokes say.
 *
 * There is no line editor on this side to ask: the shell's own is on the other end of a pty. So
 * this models the small part that matters — characters, the ways to erase them, and Enter.
 *
 * It is deliberately quick to give up. A completion, a history recall, any key it does not model
 * changes the line without saying what the line became, so the line is marked unreadable until the
 * next Enter. A line half-known is worse than a line unknown: after recalling history with the
 * arrow keys the buffer holds whatever was typed *before* the arrow, and reading a `cd` off that
 * would move the panel to a directory nobody named.
 *
 * A password typed at a prompt passes through here like anything else, and is dropped at the next
 * Enter. Nothing is kept, logged or sent anywhere but to the shell.
 */
export class TypedCommands {
  private line = "";
  private readable = true;
  private pasting = false;

  /** Feeds what was typed since the last call; returns the directory to follow, if one was named. */
  feed(data: string): string | null {
    let followed: string | null = null;

    for (let index = 0; index < data.length; index += 1) {
      const character = data[index] ?? "";

      if (this.pasting) {
        if (data.startsWith(PASTE_END, index)) {
          this.pasting = false;
          index += PASTE_END.length - 1;
          continue;
        }
        // A pasted script reaches the shell as one line, and is read as one: the separators inside
        // it are still separators, so the last `cd` in it is still the one that decides.
        this.line += character;
        continue;
      }

      if (character === "\x1b") {
        if (data.startsWith(PASTE_START, index)) {
          this.pasting = true;
          index += PASTE_START.length - 1;
          continue;
        }
        this.readable = false;
        index = skipEscape(data, index);
        continue;
      }

      if (character === "\r" || character === "\n") {
        followed = this.readable ? directoryInCommand(this.line) : null;
        this.line = "";
        this.readable = true;
        continue;
      }

      if (character === "\x7f" || character === "\b") {
        this.line = this.line.slice(0, -1);
        continue;
      }
      if (character === "\x15" || character === "\x03") {
        // Ctrl-U kills the line and Ctrl-C abandons it; the shell runs neither.
        this.line = "";
        continue;
      }
      if (character === "\x17") {
        this.line = this.line.replace(/\S+\s*$/, "");
        continue;
      }

      if (character < " ") {
        // Tab completion and the rest: the shell acts on the line and this side does not see what
        // it became.
        this.readable = false;
        continue;
      }

      this.line += character;
    }

    return followed;
  }
}

/** One command of a line, split at the separators that run commands one after another. */
function commandsIn(line: string): string[] {
  const commands: string[] = [];
  let start = 0;
  let quote: "'" | '"' | null = null;

  for (let index = 0; index < line.length; index += 1) {
    const character = line[index];
    if (quote !== null) {
      if (character === quote) {
        quote = null;
      }
      continue;
    }
    if (character === "'" || character === '"') {
      quote = character;
      continue;
    }
    // A single `&` or `|` is deliberately not a separator: what runs on the other side of one is a
    // subshell, and a `cd` in a subshell does not move the shell being followed.
    const pair = line.slice(index, index + 2);
    if (character === ";" || character === "\n" || pair === "&&" || pair === "||") {
      commands.push(line.slice(start, index));
      index += pair === "&&" || pair === "||" ? 1 : 0;
      start = index + 1;
    }
  }

  commands.push(line.slice(start));
  return commands;
}

/** What one command turned out to be. */
type Command =
  | { readonly kind: "other" }
  /** A `cd` naming a directory that can be read off the line. */
  | { readonly kind: "directory"; readonly target: string }
  /** A `cd` whose argument only the shell can work out. */
  | { readonly kind: "unreadable" };

function readCommand(command: string): Command {
  const match = /^\s*cd(?:\s+([\s\S]*?))?\s*$/.exec(command);
  if (!match) {
    return { kind: "other" };
  }
  const raw = (match[1] ?? "").trim();
  if (raw === "") {
    return { kind: "directory", target: "" };
  }

  const argument = unquote(raw);
  if (argument === null || argument.text === "") {
    // Quotes that do not close, or a quoted empty name — which is not the home directory, it is a
    // directory the shell will refuse to enter.
    return { kind: "unreadable" };
  }
  if (argument.kind === "single") {
    return { kind: "directory", target: argument.text };
  }
  if (argument.kind === "double") {
    return EXPANDS_IN_DOUBLE.test(argument.text)
      ? { kind: "unreadable" }
      : { kind: "directory", target: argument.text };
  }
  // Bare: a space is a second argument, and anything the shell expands is not a path this side can
  // name.
  if (EXPANDS_BARE.test(argument.text) || /\s/.test(argument.text)) {
    return { kind: "unreadable" };
  }
  return { kind: "directory", target: argument.text };
}

/** An argument with its quoting taken off, or null when the quotes do not close. */
function unquote(argument: string): { text: string; kind: "bare" | "double" | "single" } | null {
  const first = argument[0];
  if (first !== "'" && first !== '"') {
    return { text: argument, kind: "bare" };
  }
  if (argument.length < 2 || argument[argument.length - 1] !== first) {
    return null;
  }
  return { text: argument.slice(1, -1), kind: first === "'" ? "single" : "double" };
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

/** A CSI sequence ends at the first byte in `@`..`~`. */
function isFinalByte(character: string): boolean {
  return character >= "@" && character <= "~";
}
