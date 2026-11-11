/**
 * What the window can read back out of a terminal that is already drawn.
 *
 * The buffer belongs to the pane — it is xterm's scrollback, and only the pane has it — while the
 * caller is the assistant panel, which is about "the terminal on screen" rather than about any one
 * pane. A registry is the seam between the two: a pane registers a reader for its handle, and the
 * caller asks by handle.
 *
 * Two rules make it safe to read from a panel:
 *
 * 1. **Nothing is held that a pane did not hand over.** The reader is a closure over a live instance
 *    and is dropped when the pane unmounts, so a handle whose terminal has gone answers nothing
 *    rather than answering with stale text.
 * 2. **The read is the tail, in the pane's own words.** `translateToString(true)` is xterm's own
 *    way of saying "this row, with no trailing padding", which is the text a person sees rather
 *    than the cell grid behind it.
 */

/** Reads the last `lines` rows of one terminal, as text. */
export type TerminalBufferReader = (lines: number) => string;

const readers = new Map<string, TerminalBufferReader>();

/** Claims a handle for one pane. Registering a second reader for the same handle replaces it. */
export function registerTerminalBuffer(handle: string, read: TerminalBufferReader): () => void {
  readers.set(handle, read);
  return () => {
    if (readers.get(handle) === read) {
      readers.delete(handle);
    }
  };
}

/**
 * The last `lines` rows of a terminal, or an empty string when nothing is drawing it.
 *
 * Empty rather than a rejection: a panel that is open over a settings page has no terminal to read,
 * and "there is nothing here" is the honest answer rather than a failure the person has to dismiss.
 */
export function terminalTail(handle: string, lines: number): string {
  if (lines <= 0) {
    return "";
  }
  return readers.get(handle)?.(lines) ?? "";
}

/**
 * The last `lines` rows of a buffer, as text, with the blank rows at the end dropped.
 *
 * A terminal's buffer is longer than what it has printed: the rows under the cursor are blank until
 * something writes there, so counting back from the end would hand a caller a screenful of nothing
 * and a single line of output. The walk starts at the last row that holds text, which is the row the
 * person is looking at, and takes the window above it.
 *
 * The buffer is typed structurally rather than as xterm's own class: this is arithmetic over rows,
 * and a reader that takes a shape is one a test can hand a dozen strings to.
 */
export interface BufferLike {
  readonly length: number;
  getLine(index: number): { translateToString(trimRight?: boolean): string } | undefined;
}

export function tailOfBuffer(buffer: BufferLike, lines: number): string {
  if (lines <= 0) {
    return "";
  }
  let end = buffer.length - 1;
  while (end >= 0 && (buffer.getLine(end)?.translateToString(true) ?? "") === "") {
    end -= 1;
  }
  if (end < 0) {
    return "";
  }
  const start = Math.max(0, end - lines + 1);
  const rows: string[] = [];
  for (let index = start; index <= end; index += 1) {
    rows.push(buffer.getLine(index)?.translateToString(true) ?? "");
  }
  return rows.join("\n");
}
