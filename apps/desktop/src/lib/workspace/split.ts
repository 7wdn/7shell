/**
 * Where the line between two panes is allowed to be — stacked, or side by side.
 *
 * The size somebody dragged to is a wish, not a measurement: it was chosen in a window of some
 * size, and the window changes. What is kept is the wish; what is drawn is the wish clamped
 * against the room there is now, because the alternative — a panel taller or wider than its own
 * container — is a terminal squeezed to nothing, which looks like the terminal broke rather than
 * like the panel is too big.
 *
 * Two things apply these numbers: the divider clamps a drag to them, and each pane carries the
 * floor as its minimum height so that a window that shrinks or a size chosen in a larger window
 * takes the room out of the panes rather than out of one of them entirely. There is no reading of
 * "a terminal 40px tall" or "three file rows" in which the person meant it, and a floor is also
 * what keeps the divider reachable after a window has been made small.
 *
 * The vertical pair below is the same rule read sideways, and it is the same file because it is the
 * same rule: a divider between a list and the thing the list is used for. What differs is only
 * which of the two panes is the one being sized — the one on the left, in both panels that draw
 * this line — and how wide a pane has to be before it stops being the thing it is.
 */

/** The smallest either pane is allowed to be. */
export const MIN_PANE_PX = 160;

/**
 * The line's own thickness: `h-2` on the divider element.
 *
 * It is here so the arithmetic accounts for it — the two panes plus the line have to add up to the
 * container, and a ceiling that forgets the line is a ceiling that is 8px too generous.
 */
export const SPLIT_DIVIDER_PX = 8;

/** The height to draw the lower pane at, given how much room there is. */
export function clampPaneHeight(height: number, available: number, minPane: number = MIN_PANE_PX): number {
  if (available <= 0) {
    // Nothing has been measured yet. Only the floor applies: a guess at the ceiling would be a
    // guess about a window that is about to have a size.
    return Math.max(height, minPane);
  }
  const ceiling = Math.max(minPane, available - minPane - SPLIT_DIVIDER_PX);
  return Math.min(Math.max(height, minPane), ceiling);
}

/**
 * The smallest the pane on the left is allowed to be drawn.
 *
 * A directory tree and a list of saved commands are the same kind of thing — a name, an indent, and
 * a row of buttons that appears when the pointer is on it — and this is where that stops fitting.
 */
export const MIN_SIDE_PANE_PX = 160;

/**
 * What the pane beside it keeps, whatever the one on the left would like.
 *
 * Wider than the floor above, because the two are not the same job: the left half is a list of names
 * to choose from, and the right half is what is being read — six columns of a directory, or a line
 * somebody is editing. A panel that handed the list most of the room would be a panel whose right
 * half no longer answers the question it was opened for.
 */
export const SIDE_PANE_KEEP_PX = 280;

/**
 * The line's own thickness this way round: `w-2` on the divider element.
 *
 * Here for the reason `SPLIT_DIVIDER_PX` is: a ceiling that forgets the line is 8px too generous.
 */
export const SIDE_DIVIDER_PX = 8;

/** The width to draw the left pane at, given how much room there is. */
export function clampPaneWidth(width: number, available: number, minPane: number = MIN_SIDE_PANE_PX): number {
  if (available <= 0) {
    // Nothing has been measured yet, exactly as above: only the floor is known.
    return Math.max(width, minPane);
  }
  const ceiling = Math.max(minPane, available - SIDE_PANE_KEEP_PX - SIDE_DIVIDER_PX);
  return Math.min(Math.max(width, minPane), ceiling);
}
