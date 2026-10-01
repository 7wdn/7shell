/**
 * A sample, written the way a person reads it.
 *
 * The sizes a monitor shows are the same sizes a file browser shows, so they come from the same
 * place — `lib/files/format.ts` — rather than from a second set of units that would disagree with
 * it about what a gigabyte is. What is here is only what a monitor needs and a file list does not:
 * an uptime split into parts a locale can arrange, and a rate, which is a size with a clock
 * attached.
 */

import { formatSize } from "@/lib/files/format";

/** The dash drawn where a host did not answer. One character, one meaning, on every page. */
export const UNKNOWN = "-";

/**
 * A value as a person reads it, or the dash when there is none.
 *
 * Note what is *not* here: a zero. A zero that came from the host is a reading and is left alone;
 * a missing one is this, and the two are never drawn the same way — which is the whole reason a
 * monitor's numbers can be trusted at all.
 */
export function orUnknown(value: string | number | null | undefined): string {
  if (value === null || value === undefined || value === "") {
    return UNKNOWN;
  }
  return String(value);
}

/** A percentage, or the dash when there is nothing to take a share of. */
export function percentOf(value: number | null, digits = 0): string {
  return value === null ? UNKNOWN : `${value.toFixed(digits)}%`;
}

/** An uptime, in the parts a locale arranges: 3 days, 4 hours and 5 minutes is not `3:04:05` everywhere. */
export interface Uptime {
  readonly days: number;
  readonly hours: number;
  readonly minutes: number;
}

export function uptime(seconds: number): Uptime {
  const total = Number.isFinite(seconds) ? Math.max(0, Math.floor(seconds)) : 0;
  return {
    days: Math.floor(total / 86_400),
    hours: Math.floor((total % 86_400) / 3600),
    minutes: Math.floor((total % 3600) / 60),
  };
}

/** Bytes per second, at the same scale as everything else on screen. */
export function perSecond(bytes: number): string {
  return `${formatSize(bytes)}/s`;
}
