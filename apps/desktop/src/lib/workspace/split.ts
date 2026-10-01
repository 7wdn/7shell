/**
 * Where the line between two stacked panes is allowed to be.
 *
 * The height somebody dragged to is a wish, not a measurement: it was chosen in a window of some
 * size, and the window changes. What is kept is the wish; what is drawn is the wish clamped
 * against the room there is now, because the alternative — a panel taller than its own container —
 * is a terminal squeezed to nothing, which looks like the terminal broke rather than like the
 * panel is too big.
 *
 * Two things apply these numbers: the divider clamps a drag to them, and each pane carries the
 * floor as its minimum height so that a window that shrinks or a height chosen in a larger window
 * takes the room out of the panes rather than out of one of them entirely. There is no reading of
 * "a terminal 40px tall" or "three file rows" in which the person meant it, and a floor is also
 * what keeps the divider reachable after a window has been made small.
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
