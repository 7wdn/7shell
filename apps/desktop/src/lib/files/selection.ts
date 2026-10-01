/**
 * Which rows of a listing are selected, and where the keyboard is.
 *
 * The model is a value rather than a class of listeners, so every gesture is one function that
 * answers "and now what is selected" — which is the only way the platform's conventions can be
 * written down and tested: a plain click, a ctrl-click, a shift-click, a drag, an arrow key and a
 * directory that was read again all do something different, and none of them is obvious.
 *
 * Three things are held rather than one, and each answers a question the others cannot:
 *
 * - **`paths`** is what an action acts on.
 * - **`anchor`** is where a range starts: shift-click selects from here to there, and it deliberately
 *   does not move when the range grows — otherwise a second shift-click would select from the wrong end.
 * - **`lead`** is the row the keyboard is on. It is what a caller means by "the one row" when exactly
 *   one thing has to be chosen, and it stays put when a ctrl-click removes a row rather than moving
 *   somewhere the person did not point at.
 */
import type { SftpEntry } from "@/types/session";

export interface Selection {
  /** The selected paths, in the order the listing draws them. */
  readonly paths: readonly string[];
  /** Where a range starts, or null when nothing has been clicked yet. */
  readonly anchor: string | null;
  /** The row the keyboard is on, or null when nothing is selected. */
  readonly lead: string | null;
}

export const NOTHING_SELECTED: Selection = { paths: [], anchor: null, lead: null };

/** A plain click: this row, and only this row. */
export function selectOne(path: string): Selection {
  return { paths: [path], anchor: path, lead: path };
}

/**
 * A ctrl-click (cmd on a Mac): this row joins the selection, or leaves it.
 *
 * The anchor follows the pointer either way — a shift-click after removing a row ranges from the row
 * that was just touched, which is what the hand expects even though the row is no longer selected.
 */
export function toggle(selection: Selection, path: string): Selection {
  const without = selection.paths.filter((selected) => selected !== path);
  const paths = without.length === selection.paths.length ? [...selection.paths, path] : without;

  return { paths, anchor: path, lead: path };
}

/**
 * A shift-click: everything between where the last plain click landed and here.
 *
 * With nothing to range from — the first gesture in a directory being the shifted one — it is a plain
 * click, because "from nowhere to here" has no range to offer.
 */
export function selectRange(selection: Selection, rows: readonly string[], path: string): Selection {
  const anchor = selection.anchor;
  const from = anchor === null ? -1 : rows.indexOf(anchor);
  const to = rows.indexOf(path);
  if (from < 0 || to < 0) {
    return selectOne(path);
  }

  const [start, end] = from <= to ? [from, to] : [to, from];
  return { paths: rows.slice(start, end + 1), anchor, lead: path };
}

/** Everything the listing is showing, which is what ctrl-A means. */
export function selectAll(rows: readonly string[]): Selection {
  if (rows.length === 0) {
    return NOTHING_SELECTED;
  }
  // The ends become the anchor and the lead, so an arrow after selecting everything continues from
  // the bottom of the list rather than from wherever the pointer happened to be.
  return { paths: [...rows], anchor: rows[0] ?? null, lead: rows[rows.length - 1] ?? null };
}

/** Nothing selected, which is what clicking away from the rows means. */
export function clear(): Selection {
  return NOTHING_SELECTED;
}

/**
 * The keyboard moving the lead, with or without shift.
 *
 * Without shift the selection follows the lead — one row, the one being walked through. With shift it
 * grows from the anchor instead, which is the same range a shift-click would have made. The ends do
 * not wrap: walking off the bottom of a listing and arriving at the top is how a person loses track
 * of where they are.
 */
export function moveLead(selection: Selection, rows: readonly string[], step: number, extend: boolean): Selection {
  if (rows.length === 0) {
    return NOTHING_SELECTED;
  }

  const current = selection.lead === null ? -1 : rows.indexOf(selection.lead);
  const next = clamp(current < 0 ? (step > 0 ? 0 : rows.length - 1) : current + step, 0, rows.length - 1);
  const path = rows[next];
  if (path === undefined) {
    return selection;
  }

  return extend ? selectRange(selection, rows, path) : selectOne(path);
}

/** Home and End, which are the same move to an edge. */
export function jumpLead(
  selection: Selection,
  rows: readonly string[],
  edge: "first" | "last",
  extend: boolean,
): Selection {
  if (rows.length === 0) {
    return NOTHING_SELECTED;
  }
  const path = edge === "first" ? rows[0] : rows[rows.length - 1];
  if (path === undefined) {
    return selection;
  }

  return extend ? selectRange(selection, rows, path) : selectOne(path);
}

/**
 * What is left of a selection after the directory was read again.
 *
 * Rows that are gone drop out — a listing re-reads itself after every change, so a selection that
 * outlived its rows would let "delete these three" name something that is no longer there. The order
 * comes from the listing rather than from the old selection, so the two cannot disagree about it.
 */
export function retain(selection: Selection, rows: readonly string[]): Selection {
  const present = new Set(rows);
  const paths = rows.filter((path) => present.has(path) && selection.paths.includes(path));
  if (paths.length === selection.paths.length && paths.every((path, index) => path === selection.paths[index])) {
    // Nothing changed: answering with the same value keeps a watcher from firing for nothing.
    return selection;
  }

  return {
    paths,
    anchor: selection.anchor !== null && present.has(selection.anchor) ? selection.anchor : null,
    lead: selection.lead !== null && present.has(selection.lead) ? selection.lead : null,
  };
}

/** The entries a menu and the batch actions are about, in the order the listing draws them. */
export function selectedEntries(selection: Selection, entries: readonly SftpEntry[]): SftpEntry[] {
  const chosen = new Set(selection.paths);
  return entries.filter((entry) => chosen.has(entry.path));
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high);
}
