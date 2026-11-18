import { describe, expect, it } from "vitest";

import { filterByName } from "@/lib/files/filter";
import type { SftpEntry } from "@/types/session";

function entry(name: string, kind: SftpEntry["kind"] = "file"): SftpEntry {
  return { name, path: `/home/user/${name}`, kind, size: 0 };
}

const listing: SftpEntry[] = [
  entry("logs", "directory"),
  entry("Notes.txt"),
  entry("notes-archive.tar"),
  entry("readme.md"),
  entry("年度报告.pdf"),
];

describe("filtering a listing by name", () => {
  it("keeps the whole listing for a blank query, including the same array", () => {
    expect(filterByName(listing, "")).toBe(listing);
    expect(filterByName(listing, "   ")).toBe(listing);
  });

  it("matches part of a name without caring about case", () => {
    expect(filterByName(listing, "NOTES").map((item) => item.name)).toEqual(["Notes.txt", "notes-archive.tar"]);
    expect(filterByName(listing, "archive").map((item) => item.name)).toEqual(["notes-archive.tar"]);
  });

  it("matches folders as well as files", () => {
    expect(filterByName(listing, "log").map((item) => item.name)).toEqual(["logs"]);
  });

  it("matches names that are not ASCII", () => {
    expect(filterByName(listing, "年度").map((item) => item.name)).toEqual(["年度报告.pdf"]);
  });

  it("answers nothing when the name is not there", () => {
    expect(filterByName(listing, "missing")).toEqual([]);
  });

  it("does not match a path outside the name", () => {
    expect(filterByName([entry("notes.txt")], "home")).toEqual([]);
  });
});
