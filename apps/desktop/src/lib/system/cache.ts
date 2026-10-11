/**
 * What machine was at each address, as last seen.
 *
 * Usage memory rather than a setting, and kept the way the recent-session list is kept
 * (`lib/sessions/recent`): a session document says what somebody saved, while this says what a
 * connection happened to find — nothing a person chose, nothing with a default, and nothing a
 * settings page could explain or restore. Losing it costs a mark on a row until the next connection,
 * which is also why nothing here is worth failing a render over: unreadable storage reads as
 * "nothing remembered", and a value that no longer looks like an identity is dropped rather than
 * trusted.
 *
 * Keyed by endpoint (`user@host:port`), which is what the answer is about: the mark follows the
 * address, so editing a session to point at another machine stops drawing the old one at once, and
 * two sessions reaching one machine share a single answer.
 *
 * An answer that says nothing is not written down. "This host kept quiet" is not a fact about the
 * machine, and remembering it would overwrite what a previous connection learned — a host whose
 * `uname` answers today and whose account loses exec tomorrow is the same machine either way.
 */

import type { HostSystem } from "@/types/system";

/** Where the answers live. Versioned, because a map read by a later shape is worth discarding. */
export const SYSTEM_CACHE_KEY = "sevenshell-session-systems-v1";

/**
 * How many addresses are remembered.
 *
 * A cap rather than a policy: the whole map is a few hundred bytes per address, and the cap is what
 * keeps a script that reaches thousands of hosts from growing the WebView's storage without end.
 * The entry that goes is the one seen longest ago, since every write moves its address to the end.
 */
export const MAX_REMEMBERED_SYSTEMS = 200;

function storage(): Storage | null {
  try {
    return typeof localStorage === "undefined" ? null : localStorage;
  } catch {
    // A WebView with storage refused throws on the property itself rather than on a call, and a
    // mark on a row is not worth failing a render over.
    return null;
  }
}

/** A string field, or null for anything else: this reads storage, which is not necessarily ours. */
function text(value: unknown): string | null {
  return typeof value === "string" && value.trim() !== "" ? value : null;
}

/** One stored value, if it still looks like an identity. Anything else is dropped. */
function identity(value: unknown): HostSystem | null {
  if (!value || typeof value !== "object") {
    return null;
  }

  const stored = value as Record<string, unknown>;
  const name = text(stored.name);
  const kernel = text(stored.kernel);
  const distro = text(stored.distro);
  const distroLike = Array.isArray(stored.distroLike)
    ? stored.distroLike.map(text).filter((entry): entry is string => entry !== null)
    : [];

  // An identity with nothing in it describes no machine, so it stands in for no mark.
  if (name === null && kernel === null && distro === null) {
    return null;
  }

  return { name, kernel, distro, distroLike };
}

/**
 * Whether an identity says anything about the machine.
 *
 * The three things a mark can be drawn from, or that a person can read: an `ID_LIKE` on its own is
 * a hint about a family, not an answer about a machine, and on its own it draws nothing.
 */
export function saysNothing(system: HostSystem): boolean {
  return system.name === null && system.kernel === null && system.distro === null;
}

/** What was stored, if it still parses into a map of identities. Junk reads as "nothing remembered". */
export function parseSystemCache(raw: string | null): Record<string, HostSystem> {
  if (!raw) {
    return {};
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {};
  }

  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    return {};
  }

  const cache: Record<string, HostSystem> = {};
  for (const [endpoint, value] of Object.entries(parsed as Record<string, unknown>)) {
    const system = identity(value);
    if (endpoint.trim() !== "" && system) {
      cache[endpoint] = system;
    }
  }

  return cache;
}

/**
 * The map after one more answer: the address moves to the end and the oldest falls off the front.
 *
 * An answer that says nothing comes back as the map it was given — the same object, so a caller can
 * write it without comparing contents, and so what an earlier connection learned is not replaced by
 * a host that simply kept quiet this time.
 */
export function rememberSystem(
  cache: Readonly<Record<string, HostSystem>>,
  endpoint: string,
  system: HostSystem,
): Record<string, HostSystem> {
  if (endpoint.trim() === "" || saysNothing(system)) {
    return cache as Record<string, HostSystem>;
  }

  const next: Record<string, HostSystem> = {};
  for (const [key, value] of Object.entries(cache)) {
    if (key !== endpoint) {
      next[key] = value;
    }
  }
  next[endpoint] = system;

  const endpoints = Object.keys(next);
  for (const stale of endpoints.slice(0, Math.max(0, endpoints.length - MAX_REMEMBERED_SYSTEMS))) {
    delete next[stale];
  }

  return next;
}

/** What was remembered on this machine, read once when the window opens. */
export function readSystemCache(): Record<string, HostSystem> {
  return parseSystemCache(storage()?.getItem(SYSTEM_CACHE_KEY) ?? null);
}

/** Writes the map down. A storage that refuses the write costs the next launch its marks. */
export function writeSystemCache(cache: Readonly<Record<string, HostSystem>>): void {
  try {
    storage()?.setItem(SYSTEM_CACHE_KEY, JSON.stringify(cache));
  } catch {
    // Full or read-only storage: the marks in this run still stand, and the next launch starts empty.
  }
}
