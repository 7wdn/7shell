/**
 * The rectangle somebody drags over a listing, and which rows it covers.
 *
 * Kept apart from the pointer handling because the arithmetic is the part that can be wrong in ways a
 * screenshot never shows: a drag upwards, a rectangle that ends before it starts, a row that only
 * overlaps by a pixel, and the scroll that happens when the pointer is held past an edge.
 *
 * The rows are measured once, when the drag begins, and this works in the listing's own coordinates —
 * the distance from the top of its *contents*, not from the top of the window. That way the numbers
 * stay still while the listing scrolls under the pointer, which is exactly what a drag past the edge
 * of a long directory does.
 */

export interface Point {
  readonly x: number;
  readonly y: number;
}

/** One row, as measured: where it starts and ends down the listing. */
export interface RowBand {
  readonly path: string;
  readonly top: number;
  readonly bottom: number;
}

/** The rectangle's extent down the listing. The width does not matter: rows span it. */
export interface Band {
  readonly top: number;
  readonly bottom: number;
}

/** How close to an edge the pointer has to be before the listing starts moving. */
export const EDGE_MARGIN_PX = 24;

/** The most one pointer move may scroll, so a held pointer does not fly past what somebody wanted. */
export const MAX_SCROLL_STEP_PX = 40;

/** The band two corners make, whichever way round they were dragged. */
export function bandBetween(from: Point, to: Point): Band {
  return { top: Math.min(from.y, to.y), bottom: Math.max(from.y, to.y) };
}

/** Whether a drag is worth calling a drag: below this it is a click that moved. */
export function isDrag(from: Point, to: Point): boolean {
  return Math.abs(from.y - to.y) > 4 || Math.abs(from.x - to.x) > 4;
}

/**
 * The paths of every row the band touches.
 *
 * Touching is enough — a rectangle overlapping a row by one pixel selects it, which is what the
 * platform does and what a person aiming at the edge of a row expects.
 */
export function rowsInBand(rows: readonly RowBand[], band: Band): string[] {
  return rows.filter((row) => row.bottom > band.top && row.top < band.bottom).map((row) => row.path);
}

/**
 * How far to scroll while a drag is held near an edge, in pixels: negative is upwards.
 *
 * Proportional to how far past the margin the pointer is, so holding it a little scrolls a little and
 * holding it well past the edge scrolls faster — and zero when the pointer is anywhere in the middle,
 * which is most of the time.
 */
export function scrollStep(pointerY: number, viewport: { top: number; height: number }): number {
  const top = viewport.top + EDGE_MARGIN_PX;
  const bottom = viewport.top + viewport.height - EDGE_MARGIN_PX;

  if (pointerY < top) {
    return -Math.min(EDGE_MARGIN_PX, top - pointerY);
  }
  if (pointerY > bottom) {
    return Math.min(EDGE_MARGIN_PX, pointerY - bottom);
  }
  return 0;
}
