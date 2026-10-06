/**
 * The order the listing draws its rows in.
 *
 * The server hands over a directory already sorted — directories first, then names — and this is
 * that order written down as a rule that can be turned around: which end of a column a heading
 * starts at, what orders the rows that share a value, where a row the server said nothing about
 * goes, and what happens to the directories. It is a module of its own because none of that is
 * about a panel: it is about rows, and it is the part worth reading without a window in front of it.
 */
import { type SortDirection, byText, startsDescending } from "@/lib/sort";
import type { EntryKind, SftpEntry } from "@/types/session";

/** The columns a listing can be ordered by: the ones it carries headings for. */
export type ListingSort = "name" | "size" | "kind" | "modified" | "mode" | "owner";

/** Which of them is in use, and which end of it is being read. */
export interface Order {
  readonly column: ListingSort;
  readonly descending: boolean;
}

/**
 * The order the listing opens in.
 *
 * The name column, because that is the order the rows arrive in — the server sorts a directory by
 * name and puts the directories first — so the heading marked from the first frame is describing
 * the list on screen rather than asking somebody to press it and find out.
 */
export const INITIAL_ORDER: Order = { column: "name", descending: false };

/**
 * Where each column starts when it is first pressed.
 *
 * Text reads from the alphabet and figures from the top, which is the rule every sortable table in
 * this application follows; what "the top" means is stated per column, because it is the biggest
 * file for one, the newest time for another and the widest permissions for a third.
 */
const FIRST_DIRECTION: Record<ListingSort, SortDirection> = {
  name: "ascending",
  size: "descending",
  kind: "ascending",
  modified: "descending",
  mode: "descending",
  owner: "ascending",
};

/**
 * The order the kinds are read in, which is also the order the listing arrives in.
 *
 * By kind rather than by the word in the cell: the word is a translation, and an order that depends
 * on the language the interface happens to be in is not one two people can compare.
 */
const KIND_RANK: Record<EntryKind, number> = { directory: 0, file: 1, symlink: 2, other: 3 };

/** Pressing a heading: a column nobody has used starts at its own end, the one in use turns around. */
export function pressed(current: Order, column: ListingSort): Order {
  if (current.column === column) {
    return { column, descending: !current.descending };
  }
  return { column, descending: startsDescending(FIRST_DIRECTION[column]) };
}

/** The rows in the order they are drawn. */
export function orderEntries(entries: readonly SftpEntry[], order: Order): SftpEntry[] {
  return [...entries].sort((left, right) => compare(left, right, order));
}

function compare(left: SftpEntry, right: SftpEntry, order: Order): number {
  // A directory reads above a file whichever column is in use: a folder is a shelf rather than a
  // value, and pressing a heading turns the values around, not the shelves. It is also what the
  // server does — its listing puts the directories first and sorts the rest by name — so the
  // heading the listing opens on describes the order that is already on screen.
  const shelves = shelf(left) - shelf(right);
  if (shelves !== 0) {
    return shelves;
  }

  // A row the server sent nothing for sits at the bottom either way the column is read: an absence
  // is not the smallest value, and turning a column around must not turn "not known" into "the
  // newest" or "the widest". Three of the six columns can be missing from a listing.
  const known = [has(order.column, left), has(order.column, right)];
  if (known[0] !== known[1]) {
    return known[0] ? -1 : 1;
  }

  // Rows that share a value are ordered by name, and the name is read from the same end the column
  // is. Without that, a column whose value is the same for every row on screen — the kind column in
  // a folder of ordinary files, a size column where three files are the same size — would answer
  // the first press with the order that was already there and read as a column that does nothing.
  const values = known[0] ? byColumn(order.column, left, right) : 0;
  const compared = values !== 0 ? values : byName(left, right);
  return order.descending ? -compared : compared;
}

/** Which shelf a row stands on, compared before the column and never turned around with it. */
function shelf(entry: SftpEntry): number {
  return entry.kind === "directory" ? 0 : 1;
}

/** Whether this row has anything to say about the column: what decides the bottom of the list. */
function has(column: ListingSort, entry: SftpEntry): boolean {
  switch (column) {
    case "name":
    case "size":
    case "kind":
      // Every entry arrives with all three: a name, a kind, and a size that is zero rather than
      // absent when there is none — which is how a directory's size is reported.
      return true;
    case "modified":
      return entry.modifiedAt !== undefined;
    case "mode":
      return entry.mode !== undefined;
    case "owner":
      return entry.owner !== undefined || entry.group !== undefined;
  }
}

/** The column's own comparison. Both rows are known to have a value: `has` is what asked. */
function byColumn(column: ListingSort, left: SftpEntry, right: SftpEntry): number {
  switch (column) {
    case "name":
      return byName(left, right);
    case "size":
      return left.size - right.size;
    case "kind":
      return KIND_RANK[left.kind] - KIND_RANK[right.kind];
    case "modified":
      return (left.modifiedAt ?? 0) - (right.modifiedAt ?? 0);
    case "mode":
      return (left.mode ?? 0) - (right.mode ?? 0);
    case "owner":
      // The column shows an owner and a group. It is ordered by the owner, and by the group only
      // for the rows that have no owner to be ordered by.
      return byText(left.owner ?? left.group ?? "", right.owner ?? right.group ?? "");
  }
}

/**
 * Two rows by name.
 *
 * Lower case, because the server ordered its own listing that way: the heading the listing opens
 * with has to describe the order that is already on screen, and `A.txt` and `a.txt` are one letter
 * apart only in a case-sensitive reading. This is the name column's own comparison and every other
 * column's tie-break.
 */
function byName(left: SftpEntry, right: SftpEntry): number {
  return byText(left.name.toLowerCase(), right.name.toLowerCase());
}
