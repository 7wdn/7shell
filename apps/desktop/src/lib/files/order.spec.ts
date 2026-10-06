/**
 * The listing's own order.
 *
 * What is worth pinning here is what a window would hide: which end of a column a heading opens at,
 * where a row the server said nothing about goes, and what happens to the directories when the
 * column being read is not the name.
 *
 * The type column is ordered by the words it shows, which the panel hands over. These cases stand in
 * for those words with the plainest thing of the same shape — a file's extension — because what is
 * being checked here is the order rather than the vocabulary; the words themselves are checked where
 * they are drawn, in the panel's own cases.
 */
import { describe, expect, it } from "vitest";

import { extensionOf } from "@/lib/files/format";
import { INITIAL_ORDER, type Order, orderEntries, type TypeLabel, pressed } from "@/lib/files/order";
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

/** The type column's words, standing in for the panel's. */
const label: TypeLabel = (each) => (each.kind === "file" ? (extensionOf(each.name) ?? "") : each.kind);

/** The rows as the listing would draw them, by name. */
function drawn(entries: readonly SftpEntry[], order: Order, words: TypeLabel = label): string[] {
  return orderEntries(entries, order, words).map((each) => each.name);
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

  it("keeps the directories above the files for every column but the type", () => {
    const tiny = entry("aaa.txt", "file", { size: 1, mode: 0o777 });
    const biggest = entry("zzz.iso", "file", { size: 9_000_000, mode: 0o400 });

    // Both columns would put the folder somewhere in the middle if it were read as a file.
    expect(drawn([biggest, logs, tiny], { column: "size", descending: true })[0]).toBe("logs");
    expect(drawn([biggest, logs, tiny], { column: "size", descending: false })[0]).toBe("logs");
    expect(drawn([biggest, logs, tiny], { column: "mode", descending: false })[0]).toBe("logs");
  });

  it("puts the folders first, and turns them over with the type column", () => {
    // A directory is a type like any other in this column, and a column that could not put one type
    // below another would not be a type column: pressing it twice ends with the folders at the
    // bottom, which is what a file manager's type column does when it is turned over.
    expect(drawn(arrived, { column: "kind", descending: false })[0]).toBe("logs");
    expect(drawn(arrived, { column: "kind", descending: true }).at(-1)).toBe("logs");
  });

  it("orders the other kinds by their own words among the files", () => {
    // The words stand in for the panel's here, and they are what decides: a symlink reads under `s`,
    // so it lands between two file types rather than after all of them, and the whole list turns
    // over when the column is pressed again — folders included.
    expect(drawn(arrived, { column: "kind", descending: false })).toEqual([
      "logs",
      "photo.jpg",
      "elsewhere",
      "notes.txt",
    ]);
    expect(drawn(arrived, { column: "kind", descending: true })).toEqual([
      "notes.txt",
      "elsewhere",
      "photo.jpg",
      "logs",
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

  it("answers even when every row on screen has the same value for the column", () => {
    // A folder of ordinary files holds one type, and one press of the type column used to hand back
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
