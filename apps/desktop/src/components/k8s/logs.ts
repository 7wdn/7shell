/**
 * The bounded log buffer.
 *
 * A pod's log stream never ends on its own, so the drawer keeps the last N lines and drops what falls
 * off the top. That is a decision about memory rather than about reading: a chatty container can
 * produce tens of thousands of lines an hour, and a view that kept them all would be the reason the
 * window stopped responding.
 *
 * A timestamp is split off the line rather than rendered inside it, because whether it is shown is a
 * toggle and re-fetching the stream to answer that would be absurd: the API always sends them, and
 * the view decides whether to draw the column.
 */

/** How many lines the view keeps. Two thousand is a minute of a busy container; five is a scroll-back. */
export const LOG_LIMIT = 5000;

export interface LogLine {
  /** The timestamp the API prefixed to the line, or `""` when the line had none. */
  readonly at: string;
  readonly text: string;
}

/**
 * One line as the API sends it: `2026-11-10T09:12:33.123456789Z the message`.
 *
 * A line that does not begin with a timestamp is passed through whole — a container writing its own
 * timestamps, or a continuation of a multi-line message, must not lose its first word to a split.
 */
export function splitLogLine(line: string): LogLine {
  if (!/^\d{4}-\d\d-\d\dT/.test(line)) {
    return { at: "", text: line };
  }
  const space = line.indexOf(" ");
  return space < 0 ? { at: line, text: "" } : { at: line.slice(0, space), text: line.slice(space + 1) };
}

/**
 * The buffer with `incoming` appended, cut back to `limit` from the front.
 *
 * Pure, and returning a new array, so a caller can hand the result to a ref without the view having
 * to guess whether something changed in place.
 */
export function appendLogLines(
  buffer: readonly LogLine[],
  incoming: readonly string[],
  limit: number = LOG_LIMIT,
): LogLine[] {
  if (incoming.length === 0) {
    return [...buffer];
  }
  const next = buffer.concat(incoming.map(splitLogLine));
  return next.length > limit ? next.slice(next.length - limit) : next;
}
