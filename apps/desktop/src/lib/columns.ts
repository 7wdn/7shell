/**
 * How wide a table's columns are, and where a drag of one of them puts its edge.
 *
 * Every column carries a width: the one this build draws it at, or the one somebody moved it to. A
 * column's floor is where it stops saying anything, and there is no ceiling — a column dragged wider
 * than the panel is what the strip's own scrollbar is for, which is how a file manager behaves and
 * what somebody who drags a column to read a long name expects to happen.
 *
 * The panel's spare room goes to the columns nobody has moved, divided equally between them, and the
 * remainder of that division is handed out a pixel at a time so the columns add up to the panel
 * exactly. A column that has been moved takes no part in that: it is the width somebody put it at,
 * and the others share what is left. When every column has been moved there is nothing left to
 * divide and the panel keeps whatever is spare — the columns end where they end.
 *
 * That is what the reference implementation does with the spare room of its own grid, down to the
 * pixel-wide remainder, and it is worth saying why the shape is this one: it makes a drag legible in
 * both directions. Narrowing a column gives its room back to the columns that still share, widening
 * a column takes from them, and once the columns no longer fit, the widths are exact and the line
 * scrolls — so the boundary under the pointer is always the boundary the person is asking for.
 *
 * A module apart from any element, because the part worth checking is the arithmetic: what a column
 * is drawn at, where a drag puts it, and how wide the line has to be drawn at least. `lib/files/window.ts`
 * and `lib/workspace/split.ts` are the same kind of module for the same reason.
 */

/** One column, as far as its width is concerned. */
export interface ColumnWidth {
  readonly key: string;
  /** The width it is drawn at until somebody moves its edge, and what double-clicking it puts back. */
  readonly defaultPx: number;
  /** The narrowest it may be drawn. Below this it stops saying what it is. */
  readonly minPx: number;
}

/** What a table's columns have been moved to, by column key. Empty means nobody has moved any. */
export type MovedWidths = Readonly<Record<string, number>>;

/**
 * What a line spends on something other than its columns.
 *
 * A `<table>` spends nothing: its columns *are* its width, and a cell's own padding sits inside the
 * column. The file listing is a flex line rather than a table — cells with gaps between them, inside
 * a padded strip — so the room its columns have is the line's width less its padding and its gaps.
 * Handed in rather than guessed at, because those numbers come from the stylesheet.
 */
export interface ColumnLine {
  /** The padding on each side of the columns. */
  readonly paddingPx: number;
  /** The gap between one column and the next. */
  readonly gapPx: number;
}

/** A line that is nothing but its columns, which is what a real table's line is. */
export const NO_CHROME: ColumnLine = { paddingPx: 0, gapPx: 0 };

/** A table's columns, on the line they share. */
export interface ColumnLayout {
  readonly columns: readonly ColumnWidth[];
  readonly moved: MovedWidths;
  readonly line: ColumnLine;
}

export function layoutOf(
  columns: readonly ColumnWidth[],
  moved: MovedWidths = {},
  line: ColumnLine = NO_CHROME,
): ColumnLayout {
  return { columns, moved, line };
}

/** The narrowest a column may be drawn. Nothing at all for a key this table does not have. */
export function floorOf(layout: ColumnLayout, key: string): number {
  const column = layout.columns.find((entry) => entry.key === key);
  return column?.minPx ?? 0;
}

/**
 * The width a column asks for: what somebody moved it to, or the width this build draws it at — never
 * below its floor, because a build that raises a floor must not draw a column narrower than the floor
 * it now states.
 *
 * This is the width before the spare room is divided: what the column is drawn at is `drawnWidths`.
 */
export function widthOf(layout: ColumnLayout, key: string): number {
  const column = layout.columns.find((entry) => entry.key === key);
  if (column === undefined) {
    return 0;
  }
  return Math.max(column.minPx, layout.moved[key] ?? column.defaultPx);
}

/** What the line spends on padding and gaps rather than on columns. */
function chromeWidth(layout: ColumnLayout): number {
  return layout.line.paddingPx * 2 + layout.line.gapPx * Math.max(0, layout.columns.length - 1);
}

/** The room the columns have between them, given the width the line is drawn at. */
function columnsRoom(layout: ColumnLayout, lineWidth: number): number {
  return lineWidth - chromeWidth(layout);
}

/**
 * Every column's width as it is drawn, with the panel's spare room divided between the columns
 * nobody has moved.
 *
 * Spare room that is not divided — every column has been moved, or the panel is narrower than the
 * columns — is left where it is: the columns end where they end, and a line too wide for the panel
 * is a line that scrolls.
 *
 * A line that has not been laid out yet reports no width at all, and the honest answer then is the
 * widths themselves rather than a share of a room nobody has measured.
 */
export function drawnWidths(layout: ColumnLayout, lineWidth: number): Record<string, number> {
  const widths: Record<string, number> = {};
  let taken = 0;
  for (const column of layout.columns) {
    const width = widthOf(layout, column.key);
    widths[column.key] = width;
    taken += width;
  }

  const spare = lineWidth > 0 ? Math.floor(columnsRoom(layout, lineWidth) - taken) : 0;
  const sharing = layout.columns.filter((column) => layout.moved[column.key] === undefined);
  if (spare <= 0 || sharing.length === 0) {
    return widths;
  }

  // The remainder of the division is handed out one pixel at a time rather than dropped: the columns
  // add up to the panel exactly, which is what keeps a heading's edge and the row's edge in the same
  // place on a panel whose width is not a multiple of the columns' count.
  const each = Math.floor(spare / sharing.length);
  const remainder = spare - each * sharing.length;
  for (const [index, column] of sharing.entries()) {
    widths[column.key] = (widths[column.key] ?? 0) + each + (index < remainder ? 1 : 0);
  }
  return widths;
}

/**
 * How wide the line has to be drawn at least: the columns at the widths they ask for, and what the
 * line spends around them.
 *
 * The width below which the columns would have to give up room they were given, and therefore the
 * width at which the line scrolls instead.
 */
export function lineMinWidth(layout: ColumnLayout): number {
  const taken = layout.columns.reduce((sum, column) => sum + widthOf(layout, column.key), 0);
  return Math.ceil(chromeWidth(layout) + taken);
}

/**
 * A width as a column may be drawn at: never below its floor, and never capped. There is no ceiling
 * on a column, because the strip scrolls — which is the whole answer to "how wide may it be".
 */
export function movableWidth(px: number, minPx: number): number {
  return Math.max(Math.round(px), minPx);
}

/** Where a drag puts the edge: the width the pointer asks for, kept off the column's floor. */
export function movedTo(layout: ColumnLayout, key: string, px: number): number {
  return movableWidth(px, floorOf(layout, key));
}

/** What the width of one column is called in the stylesheet. */
function widthVar(key: string): string {
  return `--col-${key}`;
}

/**
 * What the stylesheet needs: the width of every column as it is drawn, as custom properties a heading
 * and the rows under it both read.
 *
 * One source for the two of them, which is the whole point of this shape. The heading and the cell
 * under it used to write their width twice — `w-24` over `w-20` — and were sixteen pixels apart for
 * as long as nobody looked.
 */
export function widthVars(layout: ColumnLayout, lineWidth: number): Record<string, string> {
  const vars: Record<string, string> = {};
  for (const [key, width] of Object.entries(drawnWidths(layout, lineWidth))) {
    vars[widthVar(key)] = `${width}px`;
  }
  return vars;
}
