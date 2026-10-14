/**
 * Where the little toolbar about a selection stands.
 *
 * The selection layer of xterm's renderer is a set of absolutely positioned boxes inside
 * `.xterm-selection`, one per drawn row segment, and it is the only place that knows where the
 * highlighted cells are on screen: the buffer coordinates a selection is made of would have to be
 * turned into pixels with the cell size and the scroll offset, and both of those live behind the
 * renderer's own frame. Reading the boxes is reading what the person is looking at — which is also
 * what keeps the toolbar glued to the highlight when the terminal scrolls under it.
 *
 * Two things about those boxes are worth knowing before reading the code below.
 *
 * 1. **The renderer always emits three segments** — the first line, a band across the lines in the
 *    middle, and the last line — and for a selection that covers a single line the middle band is a
 *    zero-height, full-width rectangle (it is asked for one fewer row than the two ends span, which
 *    is a negative height, and a negative height is not a length the browser will lay out). It is
 *    invisible and it is not the selection: a box built around it would reach across the whole row,
 *    and a toolbar placed from that box would stand at the far edge of the pane. Segments are
 *    therefore weighed by their own size before any of them counts.
 * 2. **The toolbar hangs from where the highlight stops** — the end of the last line it covers —
 *    rather than from the box around everything. That is the point a person's hand was at when the
 *    selection finished, and on a selection running over several lines it is the one corner that is
 *    about the end rather than about the widest line.
 *
 * The toolbar is drawn inside the pane rather than against the window, so the placement below works
 * in pane-relative coordinates; the caller measures both the boxes and the pane in client space,
 * where every box a `getBoundingClientRect` hands out lives.
 */

/** A box in client coordinates, the shape `getBoundingClientRect` reports and this module hands back. */
export interface ScreenBox {
  readonly left: number;
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
}

export interface Size {
  readonly width: number;
  readonly height: number;
}

export interface Placement {
  readonly left: number;
  readonly top: number;
}

/**
 * Where the selection is: the box around it, and the corner the toolbar hangs from.
 *
 * `end` is the far corner of the last drawn line — its `right` and `bottom` — clipped to the pane
 * like everything else, so a drag that ran off the edge cannot place the toolbar outside it.
 */
export interface SelectionShape {
  readonly box: ScreenBox;
  readonly end: { readonly right: number; readonly bottom: number };
}

/** How far the toolbar keeps below the line the selection ends on. */
export const TOOLS_GAP_PX = 6;

/**
 * How far the toolbar's left edge sits before the point the selection stopped at.
 *
 * Half of its first button, so that point lands on that button rather than on the toolbar's own
 * edge: the buttons read as growing out of the end of the highlight.
 */
export const TOOLS_ANCHOR_INSET_PX = 12;

/** How far the toolbar keeps away from the pane's own edges, so it never looks cut off. */
export const TOOLS_MARGIN_PX = 4;

/**
 * Reads the selection off the screen, or `null` when none of it is drawn.
 *
 * A highlight scrolled out of the drawn rows has no segments in the document at all, and a toolbar
 * pointing at nothing is worse than no toolbar. Nothing with any area is null as well — segments
 * that are only lines, or only holes, have nothing to stand over.
 */
export function selectionShape(surface: HTMLElement): SelectionShape | null {
  let left = Number.POSITIVE_INFINITY;
  let top = Number.POSITIVE_INFINITY;
  let right = Number.NEGATIVE_INFINITY;
  let bottom = Number.NEGATIVE_INFINITY;
  let end: { right: number; bottom: number } | null = null;

  for (const part of surface.querySelectorAll<HTMLElement>(".xterm-selection div")) {
    const box = part.getBoundingClientRect();
    // A segment with no area of its own is not part of the highlight, whatever its width: this is
    // the invisible middle band the renderer draws for a one-line selection.
    if (box.width < 1 || box.height < 1) {
      continue;
    }
    left = Math.min(left, box.left);
    top = Math.min(top, box.top);
    right = Math.max(right, box.right);
    bottom = Math.max(bottom, box.bottom);
    // The last line wins, claimed by the box rather than by the order the segments arrive in, so a
    // renderer that stopped emitting them in reading order would change nothing here.
    if (end === null || box.bottom > end.bottom || (box.bottom === end.bottom && box.right > end.right)) {
      end = { right: box.right, bottom: box.bottom };
    }
  }

  if (end === null) {
    return null;
  }

  // The segments are not clipped by their container — they are absolutely positioned children of a
  // layer that is — so a selection running past the drawn rows has to be cut back by hand.
  const bounds = surface.getBoundingClientRect();
  const clipped: ScreenBox = {
    left: Math.max(left, bounds.left),
    top: Math.max(top, bounds.top),
    right: Math.min(right, bounds.right),
    bottom: Math.min(bottom, bounds.bottom),
  };
  if (clipped.right - clipped.left < 1 || clipped.bottom - clipped.top < 1) {
    return null;
  }
  return {
    box: clipped,
    end: {
      right: Math.min(Math.max(end.right, bounds.left), bounds.right),
      bottom: Math.min(Math.max(end.bottom, bounds.top), bounds.bottom),
    },
  };
}

/**
 * Where the toolbar stands, given the selection and the pane's own box.
 *
 * Hanging from the end of the selection: its first button under the point the highlight stops at,
 * the rest of it to the right. Above the selection when the pane's floor is in the way — the rows
 * under the highlight are the ones a toolbar dropped on top of them would cover, and the highlight
 * is the thing being worked with. Both axes are clamped so that a selection at an edge keeps the
 * toolbar inside the pane rather than half of it outside.
 */
export function placeSelectionTools(shape: SelectionShape, pane: ScreenBox, tools: Size): Placement {
  const width = pane.right - pane.left;
  const height = pane.bottom - pane.top;
  const furthestLeft = Math.max(TOOLS_MARGIN_PX, width - tools.width - TOOLS_MARGIN_PX);
  const furthestTop = Math.max(TOOLS_MARGIN_PX, height - tools.height - TOOLS_MARGIN_PX);
  const below = shape.end.bottom - pane.top + TOOLS_GAP_PX;
  const top =
    below + tools.height > height - TOOLS_MARGIN_PX ? shape.box.top - pane.top - tools.height - TOOLS_GAP_PX : below;

  return {
    left: clamp(shape.end.right - pane.left - TOOLS_ANCHOR_INSET_PX, TOOLS_MARGIN_PX, furthestLeft),
    top: clamp(top, TOOLS_MARGIN_PX, furthestTop),
  };
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high);
}
