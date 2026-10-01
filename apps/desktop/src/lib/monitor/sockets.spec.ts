import { describe, expect, it } from "vitest";

import { connectionsFor, isListening, listenerRows, sortListeners, summarize } from "@/lib/monitor/sockets";
import type { SocketRow } from "@/types/monitor";

/** One socket, with the parts a test is not about filled in plausibly. */
function socket(overrides: Partial<SocketRow> = {}): SocketRow {
  return {
    protocol: "tcp",
    state: "LISTEN",
    address: "0.0.0.0",
    port: 22,
    pid: 1741,
    process: "sshd",
    peerAddress: "",
    peerPort: 0,
    sentBytes: null,
    receivedBytes: null,
    ...overrides,
  };
}

/** A connection that landed on a listener. */
function connection(overrides: Partial<SocketRow> = {}): SocketRow {
  return socket({
    state: "ESTAB",
    address: "192.168.254.133",
    peerAddress: "192.168.254.1",
    peerPort: 52344,
    ...overrides,
  });
}

describe("isListening", () => {
  it("counts a udp socket that names no peer as waiting, because it is", () => {
    expect(isListening(socket({ state: "LISTEN" }))).toBe(true);
    expect(isListening(socket({ state: "UNCONN" }))).toBe(true);
    expect(isListening(socket({ state: "ESTAB" }))).toBe(false);
    expect(isListening(socket({ state: "TIME-WAIT" }))).toBe(false);
    expect(isListening(socket({ state: "" }))).toBe(false);
  });
});

describe("connectionsFor", () => {
  const listener = socket({ address: "0.0.0.0", port: 22 });

  it("takes the connections on that port and leaves the listener itself out", () => {
    const rows = [
      listener,
      connection({ peerAddress: "10.0.0.1", peerPort: 1000 }),
      connection({ peerAddress: "10.0.0.2", peerPort: 2000, port: 80, process: "apache2" }),
    ];

    expect(connectionsFor(rows, listener).map((row) => row.peerAddress)).toEqual(["10.0.0.1"]);
  });

  /**
   * A host that listens on IPv4 and IPv6 has two rows for one service. Matching on the port alone
   * would show every connection under both of them, which reads as two connections where there is
   * one — so the family is part of what makes a connection belong to a listener.
   */
  it("keeps IPv4 and IPv6 apart", () => {
    const v4 = socket({ address: "0.0.0.0", port: 22 });
    const v6 = socket({ address: "::", port: 22, pid: 1741 });
    const rows = [
      v4,
      v6,
      connection({ address: "192.168.254.133", peerAddress: "192.168.254.1" }),
      connection({ address: "::ffff:192.168.254.133", peerAddress: "::ffff:192.168.254.1", peerPort: 52345 }),
    ];

    expect(connectionsFor(rows, v4).map((row) => row.peerAddress)).toEqual(["192.168.254.1"]);
    expect(connectionsFor(rows, v6).map((row) => row.peerAddress)).toEqual(["::ffff:192.168.254.1"]);
  });

  it("does not mix a tcp port and a udp port that happen to share a number", () => {
    const tcp = socket({ port: 53, address: "127.0.0.53", process: "systemd-resolve" });
    const rows = [tcp, connection({ port: 53, protocol: "udp", address: "127.0.0.53", peerAddress: "10.0.0.9" })];

    expect(connectionsFor(rows, tcp)).toEqual([]);
  });

  /** `ss` writes a dual-stack wildcard as `*`, and everything can be connected to one. */
  it("takes connections of either family on a wildcard that is both", () => {
    const both = socket({ address: "*", port: 80, process: "apache2" });
    const rows = [
      both,
      connection({ address: "192.168.254.133", port: 80, peerAddress: "192.168.254.1" }),
      connection({ address: "::ffff:192.168.254.133", port: 80, peerAddress: "::ffff:192.168.254.1" }),
    ];

    expect(connectionsFor(rows, both)).toHaveLength(2);
  });
});

describe("summarize", () => {
  it("counts connections and the addresses behind them, which are not the same number", () => {
    const summary = summarize([
      connection({ peerAddress: "10.0.0.1", peerPort: 1000 }),
      connection({ peerAddress: "10.0.0.1", peerPort: 1001 }),
      connection({ peerAddress: "10.0.0.2", peerPort: 1002 }),
    ]);

    expect(summary.connections).toBe(3);
    expect(summary.peers).toBe(2);
  });

  it("adds up what the connections have carried", () => {
    const summary = summarize([
      connection({ sentBytes: 1400, receivedBytes: 560 }),
      connection({ sentBytes: 600, receivedBytes: 40 }),
    ]);

    expect(summary.sentBytes).toBe(2000);
    expect(summary.receivedBytes).toBe(600);
  });

  /** `netstat` reports no counters at all, and a sum of nothing is not zero. */
  it("is null where the host reported nothing, and zero where it reported zero", () => {
    expect(summarize([connection()]).sentBytes).toBeNull();
    expect(summarize([connection({ sentBytes: 0, receivedBytes: 0 })]).sentBytes).toBe(0);
    expect(summarize([]).receivedBytes).toBeNull();
  });
});

describe("listenerRows", () => {
  it("lists what is waiting, each with what it is carrying", () => {
    const rows = listenerRows([
      socket({ port: 22, address: "0.0.0.0" }),
      socket({ port: 631, address: "127.0.0.1", process: "cupsd", pid: 1752 }),
      connection({ port: 22, sentBytes: 1400, receivedBytes: 560 }),
      connection({ port: 22, peerAddress: "10.0.0.7", peerPort: 51000 }),
    ]);

    expect(rows.map((row) => row.socket.port)).toEqual([22, 631]);
    expect(rows[0]?.summary.connections).toBe(2);
    expect(rows[0]?.summary.peers).toBe(2);
    expect(rows[0]?.summary.sentBytes, "only the connection that reported one").toBe(1400);
    expect(rows[1]?.summary.connections).toBe(0);
  });
});

describe("sortListeners", () => {
  const rows = listenerRows([
    socket({ port: 631, address: "127.0.0.1", process: "cupsd", pid: 1752 }),
    socket({ port: 22, address: "0.0.0.0", process: "sshd", pid: 1 }),
    socket({ port: 80, address: "::", process: "apache2", pid: 1787 }),
    connection({ port: 22 }),
  ]);

  it("orders by port, either way", () => {
    expect(sortListeners(rows, "port", false).map((row) => row.socket.port)).toEqual([22, 80, 631]);
    expect(sortListeners(rows, "port", true).map((row) => row.socket.port)).toEqual([631, 80, 22]);
  });

  it("orders by the listen address in one order on every machine", () => {
    expect(sortListeners(rows, "address", false).map((row) => row.socket.address)).toEqual([
      "0.0.0.0",
      "127.0.0.1",
      "::",
    ]);
  });

  it("orders by how busy a port is, and leaves the list it was given alone", () => {
    const before = rows.map((row) => row.socket.port);

    // Ties keep the port order rather than being reversed with the column, so two ports that are
    // equally busy stay in an order somebody can predict.
    expect(sortListeners(rows, "connections", true).map((row) => row.socket.port)).toEqual([22, 80, 631]);
    expect(
      rows.map((row) => row.socket.port),
      "sorting is not editing",
    ).toEqual(before);
  });
});
