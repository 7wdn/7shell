/**
 * What a `#n` in a saved line means, and how a line is filled in.
 *
 * Here rather than in the store because two callers need it: the panel that fills a line before
 * sending it, and the menu that fills one before inserting it into the input bar. The rule is the
 * same in both places, and a second copy of it would be a second answer to what `#1` is.
 */

/** How many `#n` slots a line may use. The same five the storage knows about. */
export const SLOTS = 5;

/** The slots a line uses, in order: `#1` in `service #1 restart` means `[1]`. */
export function slotsIn(command: string): number[] {
  const found = new Set<number>();
  for (const match of command.matchAll(/#([1-9])/g)) {
    const slot = Number(match[1]);
    if (slot <= SLOTS) {
      found.add(slot);
    }
  }
  return [...found].sort((left, right) => left - right);
}

/**
 * The line with its slots filled in.
 *
 * Substituted raw, the way the value would have been typed: these are shell lines, and a value
 * somebody wants quoted is one they quote. Slots left empty stay as they are, so a half-filled
 * command is visibly not a finished one rather than quietly something else.
 */
export function fillSlots(command: string, values: readonly string[]): string {
  return command.replace(/#([1-9])/g, (whole, digit: string) => {
    const value = values[Number(digit) - 1];
    return value === undefined || value.trim() === "" ? whole : value;
  });
}
