/**
 * How a sortable table reads its columns.
 *
 * One module rather than a copy per table: everything in this application that sorts a column sorts
 * the same way, and the way it sorts is a rule rather than a detail — text reads from the alphabet,
 * figures from the top, and pressing a heading that is already in use turns the order around.
 */

/** Which end of a column a table reads from. */
export type SortDirection = "ascending" | "descending";

/**
 * Two strings, in the one order that is the same on every machine.
 *
 * Code units rather than `localeCompare`: an order that depends on the machine's locale is an order
 * two people describe differently, and none of the columns here holds words in anybody's language —
 * addresses, mount points and process names are identifiers.
 */
export function byText(left: string, right: string): number {
  if (left === right) {
    return 0;
  }
  return left < right ? -1 : 1;
}

/**
 * Where a column starts when it is first chosen.
 *
 * Stated per column by the table, because "descending" means different things to a name and to a
 * number: a column of figures is opened showing the biggest, and a column of addresses from the
 * alphabet. This turns that into the flag the sort itself wants.
 */
export function startsDescending(first: SortDirection): boolean {
  return first === "descending";
}
