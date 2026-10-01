/**
 * Where the keyboard goes next in an open menu.
 *
 * Separators and rows that are greyed out are stepped over rather than landed on: arrow keys that
 * stopped on a line nobody can press would read as a menu that had stopped working. A row that opens
 * a submenu is a destination like any other — it is something the keyboard can act on. The ends wrap
 * around, which is what a menu does everywhere else.
 */
import { type MenuEntry, canFocus } from "./types";

/**
 * The index `step` entries away from `from` that the keyboard can land on, or null when there is
 * nothing to land on at all.
 *
 * `from` is allowed to sit outside the list, which is what makes the two edge cases below read as
 * one rule rather than two.
 */
export function nextEnabledIndex(entries: readonly MenuEntry[], from: number, step: 1 | -1): number | null {
  for (let offset = 1; offset <= entries.length; offset += 1) {
    const index = wrap(from + step * offset, entries.length);
    const entry = entries[index];
    if (entry && canFocus(entry)) {
      return index;
    }
  }
  return null;
}

/** The first entry the keyboard can land on, for Home and for the first press of ArrowDown. */
export function firstEnabledIndex(entries: readonly MenuEntry[]): number | null {
  return nextEnabledIndex(entries, -1, 1);
}

/** The last one, for End. */
export function lastEnabledIndex(entries: readonly MenuEntry[]): number | null {
  return nextEnabledIndex(entries, entries.length, -1);
}

function wrap(value: number, count: number): number {
  return ((value % count) + count) % count;
}
