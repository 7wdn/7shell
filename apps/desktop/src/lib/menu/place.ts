/**
 * Where an open menu is drawn.
 *
 * A pointer near an edge is the ordinary case rather than the exception — it goes wherever the thing
 * somebody right-clicked happens to be — so the menu opens at the pointer when it fits there and
 * against the pointer when it does not. It is flipped above the line it was opened on before it is
 * pushed sideways, because the row under the pointer is the one being aimed at and a menu that slid
 * away from it would lose the connection between the two.
 */

export interface Point {
  readonly x: number;
  readonly y: number;
}

export interface Size {
  readonly width: number;
  readonly height: number;
}

/** Where a row sits on screen, which is what a submenu is placed against. */
export interface Rect {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

export interface Placement {
  readonly left: number;
  readonly top: number;
}

/** The gap kept between the menu and the edge of the window, so it never looks cut off. */
export const MENU_MARGIN_PX = 8;

export function placeMenu(anchor: Point, size: Size, viewport: Size, margin = MENU_MARGIN_PX): Placement {
  // The far corner is a floor as well as a ceiling: a menu taller than the window would otherwise be
  // pushed off the top edge by its own height and lose its first entries.
  const furthestX = Math.max(margin, viewport.width - size.width - margin);
  const furthestY = Math.max(margin, viewport.height - size.height - margin);
  const top = anchor.y + size.height + margin > viewport.height ? anchor.y - size.height : anchor.y;

  return { left: clamp(anchor.x, margin, furthestX), top: clamp(top, margin, furthestY) };
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high);
}

/**
 * Where a submenu is drawn: against the row that opened it.
 *
 * Beside the row rather than at the pointer, because the list belongs to that row and a list that
 * appeared somewhere else on screen would have to be matched back to it by eye. It opens to the
 * right and folds over to the left when the window has no room — the row itself stays visible either
 * way, which is the point: it is the thing being read while the list is chosen from.
 */
export function placeSubmenu(parent: Rect, size: Size, viewport: Size, margin = MENU_MARGIN_PX): Placement {
  const beside = parent.left + parent.width;
  const left = beside + size.width + margin > viewport.width ? parent.left - size.width : beside;
  const furthestX = Math.max(margin, viewport.width - size.width - margin);
  const furthestY = Math.max(margin, viewport.height - size.height - margin);

  return { left: clamp(left, margin, furthestX), top: clamp(parent.top, margin, furthestY) };
}
