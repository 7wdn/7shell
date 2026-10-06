/**
 * The listing's own order.
 *
 * What is worth pinning here is what a window would hide: which end of a column a heading opens at,
 * where a row the server said nothing about goes, and what happens to the directories when the
 * column being read is not the name.
 */
import { describe, expect, it } from "vitest";

import { INITIAL_ORDER, type Order, orderEntries, pressed } from "@/lib/files/order";
import type { SftpEntry } from "@/types/session";

function entry(name: string, kind: SftpEntry["kind"], rest: Partial<SftpEntry> = {}): SftpEntry {
  return { name, path: `/home/user/${name}`, kind, size: 0, ...rest };
}

const logs = entry("logs", "directory", { modifiedAt: 1_600_000_000, mode: 0o755, owner: "labuser", group: "labuser" });
const notes = entry("notes.txt", "file", {
  size: 900,
  modifiedAt: 1_700_000_000,
  mode: 0o644,
  owner: "labuser",
  group: "labuser",
});
const photo = entry("photo.jpg", "file", {
  size: 5_000,
  modifiedAt: 1_750_000_000,
  mode: 0o600,
  owner: "root",
  group: "root",
});
const elsewhere = entry("elsewhere", "symlink", {
  size: 12,
  modifiedAt: 1_650_000_000,
  mode: 0o777,
  owner: "root",
  group: "root",
});

/** The rows as the listing would draw them, by name. */
function drawn(entries: readonly SftpEntry[], order: Order): string[] {
  return orderEntries(entries, order).map((each) => each.name);
}

describe("the order a listing opens in", () => {
  it("is the order the server sent: directories first, then names", () => {
    // Deliberately not in that order on the way in, because echoing what arrived is not the claim.
    const arrived = [notes, photo, logs, elsewhere];

    expect(drawn(arrived, INITIAL_ORDER)).toEqual(["logs", "elsewhere", "notes.txt", "photo.jpg"]);
  });

  it("reads a name the way the server reads it, whatever case it is written in", () => {
    const upper = entry("Zebra.txt", "file");
    const lower = entry("apple.txt", "file");

    expect(drawn([upper, lower], INITIAL_ORDER), "code units would put a capital Z first").toEqual([
      "apple.txt",
      "Zebra.txt",
    ]);
  });
});

describe("pressing a heading", () => {
  it("opens a column nobody has used at the end it is read from", () => {
    expect(pressed(INITIAL_ORDER, "size")).toEqual({ column: "size", descending: true });
    expect(pressed(INITIAL_ORDER, "modified")).toEqual({ column: "modified", descending: true });
    expect(pressed(INITIAL_ORDER, "mode")).toEqual({ column: "mode", descending: true });
    expect(pressed(INITIAL_ORDER, "owner")).toEqual({ column: "owner", descending: false });
  });

  it("turns the column already in use around", () => {
    const bySize = pressed(INITIAL_ORDER, "size");

    expect(pressed(bySize, "size")).toEqual({ column: "size", descending: false });
  });

  it("starts the name column from the alphabet, which is where the listing already was", () => {
    expect(pressed(INITIAL_ORDER, "name"), "pressing the heading the listing opened on reverses it").toEqual({
      column: "name",
      descending: true,
    });
    expect(pressed({ column: "size", descending: true }, "name")).toEqual({ column: "name", descending: false });
  });
});

describe("the rows in that order", () => {
  const arrived = [logs, notes, photo, elsewhere];

  it("reads the biggest file first when the size column is chosen", () => {
    expect(drawn(arrived, { column: "size", descending: true })).toEqual([
      "logs",
      "photo.jpg",
      "notes.txt",
      "elsewhere",
    ]);
    expect(drawn(arrived, { column: "size", descending: false })).toEqual([
      "logs",
      "elsewhere",
      "notes.txt",
      "photo.jpg",
    ]);
  });

  it("reads the newest time first, and the widest permissions first", () => {
    expect(drawn(arrived, { column: "modified", descending: true })).toEqual([
      "logs",
      "photo.jpg",
      "notes.txt",
      "elsewhere",
    ]);
    expect(drawn(arrived, { column: "modified", descending: false })).toEqual([
      "logs",
      "elsewhere",
      "notes.txt",
      "photo.jpg",
    ]);
    expect(drawn(arrived, { column: "mode", descending: true })).toEqual([
      "logs",
      "elsewhere",
      "notes.txt",
      "photo.jpg",
    ]);
    expect(drawn(arrived, { column: "mode", descending: false })).toEqual([
      "logs",
      "photo.jpg",
      "notes.txt",
      "elsewhere",
    ]);
  });

  it("keeps the directories above the files whichever column is being read", () => {
    const tiny = entry("aaa.txt", "file", { size: 1, mode: 0o777 });
    const biggest = entry("zzz.iso", "file", { size: 9_000_000, mode: 0o400 });

    // Both columns would put the folder somewhere in the middle if it were read as a file.
    expect(drawn([biggest, logs, tiny], { column: "size", descending: true })[0]).toBe("logs");
    expect(drawn([biggest, logs, tiny], { column: "size", descending: false })[0]).toBe("logs");
    expect(drawn([biggest, logs, tiny], { column: "mode", descending: false })[0]).toBe("logs");
  });

  it("reads the kind column in kind order rather than in the language's order", () => {
    expect(drawn(arrived, { column: "kind", descending: false })).toEqual([
      "logs",
      "photo.jpg",
      "notes.txt",
      "elsewhere",
    ]);
    expect(drawn(arrived, { column: "kind", descending: true })).toEqual([
      "logs",
      "elsewhere",
      "notes.txt",
      "photo.jpg",
    ]);
  });

  it("groups files by their type rather than by their name", () => {
    // What somebody means by "sort by type" in a folder of installers: everything of one type
    // together. Name order would be aaa, mmm, notype, zzz — which is what it looked like before.
    const deb = entry("aaa.deb", "file");
    const rpm = entry("zzz.rpm", "file");
    const tarball = entry("mmm.tar.gz", "file");
    const plain = entry("notype", "file");

    expect(drawn([deb, rpm, tarball, plain], { column: "kind", descending: false })).toEqual([
      "notype",
      "aaa.deb",
      "mmm.tar.gz",
      "zzz.rpm",
    ]);
    expect(drawn([deb, rpm, tarball, plain], { column: "kind", descending: true })).toEqual([
      "zzz.rpm",
      "mmm.tar.gz",
      "aaa.deb",
      "notype",
    ]);
  });

  it("does not turn the folders around with the column, even the one that is about kinds", () => {
    // Descending kinds would start with the sockets and the links, and the folders would be at the
    // bottom of the list somebody keeps their work in.
    expect(drawn(arrived, { column: "kind", descending: true })[0]).toBe("logs");
  });

  it("answers even when every row on screen has the same value for the column", () => {
    // A folder of ordinary files holds one kind, and one press of the kind column used to hand back
    // the order the rows arrived in — which is the order on screen — so the column read as one that
    // does nothing at all. The same holds for three files of one size under the size column.
    const first = entry("a.txt", "file", { size: 4, modifiedAt: 1 });
    const second = entry("b.txt", "file", { size: 4, modifiedAt: 1 });

    expect(drawn([first, second], { column: "kind", descending: false })).toEqual(["a.txt", "b.txt"]);
    expect(drawn([first, second], { column: "kind", descending: true })).toEqual(["b.txt", "a.txt"]);
    expect(drawn([first, second], { column: "size", descending: true })).toEqual(["b.txt", "a.txt"]);
    expect(drawn([first, second], { column: "size", descending: false })).toEqual(["a.txt", "b.txt"]);
  });

  it("orders by the owner, and by the group only where there is no owner", () => {
    const owned = [notes, photo];

    expect(drawn(owned, { column: "owner", descending: false })).toEqual(["notes.txt", "photo.jpg"]);
    expect(drawn(owned, { column: "owner", descending: true })).toEqual(["photo.jpg", "notes.txt"]);
  });

  it("puts a row the server said nothing about at the bottom, either way the column is read", () => {
    const undated = entry("undated.txt", "file", { size: 10 });

    for (const descending of [false, true]) {
      for (const column of ["modified", "mode", "owner"] as const) {
        expect(
          drawn([undated, notes, photo], { column, descending }).at(-1),
          `${column}, descending=${descending}: a row with nothing to read is not the smallest value`,
        ).toBe("undated.txt");
      }
    }
  });
});
