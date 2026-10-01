/**
 * The host's sockets, arranged the way the network page reads them.
 *
 * The far side answers one list — every socket, listening and connected — because that is what the
 * two tools that report them produce and asking twice for facts already answered together would be
 * two commands for one question. What a page wants is two views of that list: the ports that are
 * waiting for something, and, under the one somebody picked, the connections it is carrying.
 *
 * Three rules run through this, and each is a place where a plausible shortcut is wrong:
 *
 * 1. **A connection belongs to a listener of its own family.** A host that runs sshd on IPv4 and
 *    IPv6 has two listening sockets on port 22, and a connection accepted by one of them is on the
 *    same address family as that one. Matching on the port alone would show every connection twice
 *    — under both rows — which reads as two connections where there is one.
 * 2. **A counter nobody reported is not a zero.** `ss` reports what a socket has carried and
 *    `netstat` does not, so a total is `null` when nothing answered rather than `0`, which would
 *    mean "this service has carried nothing" about a service nobody measured.
 * 3. **`UNCONN` is listening.** UDP accepts nothing and so has no listening state; a UDP socket
 *    that names no peer is waiting for datagrams, which is the same thing to a reader.
 */

import { byText } from "@/lib/monitor/sort";
import type { SocketRow } from "@/types/monitor";

/** What one listening socket is carrying right now. */
export interface ListenerSummary {
  /** How many connections are attached to it. */
  readonly connections: number;
  /** How many distinct addresses they come from, which is not the same number. */
  readonly peers: number;
  /** What they have carried in total, or `null` when the host reported no counters at all. */
  readonly sentBytes: number | null;
  readonly receivedBytes: number | null;
}

/** One line of the page's table: a socket that is waiting, and what it is carrying. */
export interface ListenerRow {
  readonly socket: SocketRow;
  readonly summary: ListenerSummary;
}

/** The columns the table can be ordered by. */
export type ListenerSort = "address" | "port" | "connections" | "process";

/** Whether a socket is waiting for something rather than talking to something. */
export function isListening(row: SocketRow): boolean {
  return row.state === "LISTEN" || row.state === "UNCONN";
}

/**
 * The connections one listening socket is carrying.
 *
 * Matched by protocol, by port and by address family — see the module comment for why all three,
 * and in particular why the family is not optional on a host that listens on both.
 */
export function connectionsFor(rows: readonly SocketRow[], listener: SocketRow): SocketRow[] {
  return rows.filter(
    (row) =>
      !isListening(row) &&
      row.protocol === listener.protocol &&
      row.port === listener.port &&
      sameFamily(row.address, listener.address),
  );
}

/** Whether two addresses are of the same family, by the only thing that distinguishes them here. */
function sameFamily(address: string, listener: string): boolean {
  // `ss` spells the wildcard of a dual-stack socket `*`, and a connection to one can be of either
  // family — which is the whole point of it. Reading that as its own family would hide every
  // connection to such a port, and the page would say a busy service had nothing on it.
  if (address === "*" || listener === "*") {
    return true;
  }
  return address.includes(":") === listener.includes(":");
}

/** What a set of connections adds up to. */
export function summarize(connections: readonly SocketRow[]): ListenerSummary {
  const peers = new Set(connections.map((row) => row.peerAddress).filter((address) => address !== ""));
  const sent = connections.map((row) => row.sentBytes).filter((bytes): bytes is number => bytes !== null);
  const received = connections.map((row) => row.receivedBytes).filter((bytes): bytes is number => bytes !== null);

  return {
    connections: connections.length,
    peers: peers.size,
    // Summed over the ones that answered, or nothing at all when none did: a partial sum shown as
    // a total would be worse than a dash, because it looks complete.
    sentBytes: sent.length === 0 ? null : sent.reduce((total, bytes) => total + bytes, 0),
    receivedBytes: received.length === 0 ? null : received.reduce((total, bytes) => total + bytes, 0),
  };
}

/** Every listening socket, each with what it is carrying, in the order the host listed them. */
export function listenerRows(rows: readonly SocketRow[]): ListenerRow[] {
  return rows.filter(isListening).map((socket) => ({ socket, summary: summarize(connectionsFor(rows, socket)) }));
}

/** The same rows, in the order somebody asked for. */
export function sortListeners(rows: readonly ListenerRow[], sort: ListenerSort, descending: boolean): ListenerRow[] {
  const direction = descending ? -1 : 1;
  return [...rows].sort((left, right) => {
    const compared = compare(left, right, sort);
    // The port breaks every tie, so two rows that read the same keep one order between samples
    // instead of swapping places every few seconds.
    return compared !== 0 ? compared * direction : left.socket.port - right.socket.port;
  });
}

function compare(left: ListenerRow, right: ListenerRow, sort: ListenerSort): number {
  switch (sort) {
    case "address":
      // Code units rather than `localeCompare`, and the reason is visible in the column: a locale
      // that treats punctuation as lighter than digits sorts `::` above `0.0.0.0`, so the table
      // would be in one order for one person and another for the next. Addresses are not words.
      return byText(left.socket.address, right.socket.address) || left.socket.port - right.socket.port;
    case "connections":
      return left.summary.connections - right.summary.connections;
    case "process":
      return byText(left.socket.process, right.socket.process);
    default:
      return left.socket.port - right.socket.port;
  }
}
