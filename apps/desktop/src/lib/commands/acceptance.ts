/**
 * What somebody has taken from the completion list before.
 *
 * A hint and not a ranking of its own: each taking is worth a little, the total is capped well below
 * the distance between two match qualities, and nothing is written down. "What I have been reaching
 * for just now" is the whole point — a list that remembered last month would be a list that argues
 * with what somebody is doing today, and one that outranked a better match would be a list that
 * hides what they typed.
 */

/** What one taking is worth, and how much the whole memory may add up to. */
const PER_TAKING = 60;
const MAX_BOOST = 300;

/** How many candidates are remembered: past this the ones nobody reaches for fall off. */
const REMEMBERED = 256;

const taken = new Map<string, number>();

/** Notes that somebody took this row: what it inserts, which is what a later list matches on. */
export function recordAccepted(insert: string): void {
  if (insert === "") {
    return;
  }

  taken.set(insert, (taken.get(insert) ?? 0) + 1);
  if (taken.size > REMEMBERED) {
    // The oldest insertion, which is the one that has not been reached for in the longest time.
    const oldest = taken.keys().next();
    if (!oldest.done) {
      taken.delete(oldest.value);
    }
  }
}

/** What taking this row before is worth on the score, or 0 for one nobody has taken. */
export function acceptanceBoost(insert: string): number {
  return Math.min(MAX_BOOST, (taken.get(insert) ?? 0) * PER_TAKING);
}

/** Forgets everything, which is what a test needs and nothing else does. */
export function forgetAccepted(): void {
  taken.clear();
}
