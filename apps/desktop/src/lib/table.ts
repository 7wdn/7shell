/**
 * What a table looks like, in one place.
 *
 * Ten tables across the host pages and the monitor panel had drifted into three treatments of the
 * headings, two treatments of the rows and two hover shades — the same control learned twice, and a
 * table that did not answer the pointer read as a picture of a table rather than a table. So the
 * vocabulary lives here and the components wear it, and `scripts/tests/tables.test.mjs` fails the
 * day one of them goes back to writing its own.
 *
 * Four rules, all of them earned:
 *
 * 1. **A row always answers the pointer**, whether or not it can be pressed. A row is a thing with
 *    readings in it, and the band under the cursor is how somebody knows which row they are reading.
 *    Being pressable is a separate question, and that is what `clickable` adds: the pointer and a
 *    focus ring for the keyboard. (The file listing is not a table — it is a multi-select list with
 *    its own roles and its own keyboard — so it answers to its own rules.)
 * 2. **Two surfaces, two densities.** `page` is a page's own table: 12px, headings pinned over the
 *    rows that scroll under them, and a rule under every row. `panel` is the monitor panel's: 10px,
 *    short enough that nothing needs pinning, and no rule between the rows — at that size a hairline
 *    under every reading is more furniture than the readings are worth.
 * 3. **Empty is a row of the table.** A heading over nothing is what a collapsed list looks like, so
 *    the message spans the table's own columns instead of floating under it.
 * 4. **A heading that sorts is `ui/SortHeader`.** No table writes its own arrows.
 *
 * Everything a page draws its own way — alignment, which cells are monospaced, what is truncated and
 * what carries a `title` — stays in the component: this file is about the shape, not about the
 * columns. A column's *width* was in that list and is not any more: a width is read by the heading
 * and by every row under it, and an edge of it can be dragged, so the two copies it used to be
 * written as drifted apart. `lib/columns.ts` is where a width comes from now.
 */

import { cn } from "@/lib/utils";

/** The two surfaces a table is drawn on. */
export type TableSurface = "page" | "panel";

/** The parts of a table, as the classes each one wears. */
export interface TableParts {
  /** The `<table>` itself: the size this surface is read at. */
  readonly table: string;
  /** The `<thead>`. Pinning is a surface's decision; the panel's tables are too short to need it. */
  readonly head: string;
  /** The row of headings. */
  readonly headRow: string;
  /** One heading. */
  readonly headCell: string;
  /** The rule under a row of readings — which the panel's rows do not draw. */
  readonly row: string;
  /** One cell of a row of readings. */
  readonly cell: string;
  /** The one cell of a message row, which spans every column. */
  readonly emptyCell: string;
}

export const TABLE: Readonly<Record<TableSurface, TableParts>> = {
  page: {
    table: "w-full border-collapse text-xs",
    // Pinned, and opaque rather than tinted: a row scrolling under a translucent strip is read
    // through it, as two lines of text written on top of each other.
    head: "sticky top-0 bg-background",
    headRow: "border-b border-border text-muted-foreground",
    headCell: "py-1 pr-2 font-medium last:pr-0",
    row: "border-b border-border/50",
    cell: "py-1 pr-2 last:pr-0",
    emptyCell: "py-3 text-muted-foreground",
  },
  panel: {
    table: "w-full table-fixed border-collapse text-[10px]",
    head: "",
    headRow: "border-b border-border text-muted-foreground",
    headCell: "px-1 py-1 font-medium first:pl-0 last:pr-0",
    row: "",
    cell: "px-1 py-0.5 first:pl-0 last:pr-0",
    emptyCell: "px-1 py-1 text-muted-foreground first:pl-0",
  },
};

/** What a row is, beyond the surface: whether it can be pressed, and whether it is the one on show. */
export interface RowState {
  /** Pressable rows get the pointer and a focus ring; the rest only get the band. */
  readonly clickable?: boolean;
  /**
   * The row the view is about — the process the detail beside it describes, the port whose
   * connections are listed under it. It stays marked under the pointer rather than being replaced
   * by the hover band, which is why the two states have their own shade.
   */
  readonly selected?: boolean;
}

/** One row of readings: the surface's rule under it, and how it answers the pointer. */
export function rowClass(surface: TableSurface, state: RowState = {}): string {
  return cn(
    TABLE[surface].row,
    "transition-colors",
    state.selected ? "bg-accent/60 hover:bg-accent/70" : "hover:bg-accent/50",
    state.clickable && "cursor-pointer outline-none focus-visible:bg-accent/60",
  );
}
