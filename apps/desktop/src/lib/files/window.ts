import type { RowBand } from "./marquee";

/**
 * Which rows of a list are worth drawing.
 *
 * A directory can hold fifty thousand entries and a screen shows thirty. Drawing all of them costs
 * a row element each — thirty thousand of them is seconds of layout and hundreds of megabytes — and
 * none of it is ever looked at. So the list draws the rows somebody can see, plus a few either side
 * so a fast scroll does not show a gap, and two spacers stand in for the rest so the scrollbar keeps
 * telling the truth about how long the listing is.
 *
 * This is the arithmetic on its own, apart from any element: it is the part that has to be right at
 * the ends of the list and at every scroll position, and it is the part that can be checked without
 * a browser.
 *
 * Rows are assumed to be one height, which is what this listing is: a name, a size, a date, all on a
 * single line. A listing whose rows wrapped would need measuring each one, and that is a different
 * component rather than a flag on this function.
 */
export interface RowWindow {
  /** The index of the first row to draw. */
  readonly first: number;
  /** One past the last row to draw, which is `Array.prototype.slice`'s own convention. */
  readonly last: number;
  /** How tall the spacer above the drawn rows is, in pixels. */
  readonly before: number;
  /** How tall the spacer below them is. */
  readonly after: number;
}

/**
 * Where every row of a listing sits, from one row's position and the height of a row.
 *
 * This is what a rectangle dragged over the listing is matched against, and it has to cover rows the
 * document does not hold: a drag held past an edge scrolls, so the rows a rectangle should take in
 * are exactly the ones not drawn yet. Arithmetic is the only thing that knows where they would be.
 */
export function rowBands(paths: readonly string[], start: number, pitch: number): RowBand[] {
  if (pitch <= 0) {
    return [];
  }

  return paths.map((path, index) => {
    const top = start + index * pitch;
    return { path, top, bottom: top + pitch };
  });
}

export interface RowWindowRequest {
  /** How far the container is scrolled, in content coordinates. */
  readonly scrolled: number;
  /** How tall the container is, in pixels. */
  readonly viewport: number;
  /** How tall one row is, in pixels. `0` means the rows have not been measured yet. */
  readonly pitch: number;
  /** How many rows there are. */
  readonly count: number;
  /** How many rows beyond the visible ones to draw, so a scroll does not catch an empty strip. */
  readonly overscan: number;
}

/**
 * How many rows to draw when no row has been measured yet.
 *
 * A panel that has never been laid out — one inside a tab that has not been shown, or one whose
 * styles have not arrived yet — reports no height at all, and drawing nothing would leave it looking
 * empty until something happened to measure it. A screenful is the answer that is either right or
 * about to be corrected.
 */
export const UNMEASURED_ROWS = 40;

export function rowWindow({ scrolled, viewport, pitch, count, overscan }: RowWindowRequest): RowWindow {
  if (count <= 0) {
    return { first: 0, last: 0, before: 0, after: 0 };
  }
  if (pitch <= 0) {
    // Without a row height there is no honest spacer either: the content is as tall as the rows
    // drawn until the first measurement says otherwise.
    return { first: 0, last: Math.min(count, UNMEASURED_ROWS), before: 0, after: 0 };
  }

  // A container can be measured before it is laid out — a panel that has not been shown yet is
  // nought pixels tall — and the honest answer then is "draw a screenful", not "draw nothing".
  const height = viewport > 0 ? viewport : pitch;

  const first = clamp(Math.floor(Math.max(0, scrolled) / pitch) - overscan, 0, count);
  const last = clamp(Math.ceil((Math.max(0, scrolled) + height) / pitch) + overscan, first, count);

  return { first, last, before: first * pitch, after: (count - last) * pitch };
}

function clamp(value: number, lowest: number, highest: number): number {
  return Math.min(Math.max(value, lowest), highest);
}
