/**
 * The table from a file to its drawing.
 *
 * The drawings themselves are assets, not something this suite can inspect for meaning; what it can
 * prove is that every key the table names has a file behind it, that a whole name beats its
 * extension where it should, and that the rows with no type of their own keep the generic glyph.
 */
import { describe, expect, it } from "vitest";

import { AVAILABLE_FILE_ICONS, USED_FILE_ICONS, fileIconMarkup } from "@/lib/files/icons";

describe("the table from a name to a drawing", () => {
  it("has a drawing behind every key it names", () => {
    expect(USED_FILE_ICONS.filter((key) => !AVAILABLE_FILE_ICONS.includes(key))).toEqual([]);
  });

  it("reads an extension without caring about case", () => {
    expect(fileIconMarkup("holiday.JPG", "file")).toBe(fileIconMarkup("holiday.jpg", "file"));
    expect(fileIconMarkup("holiday.jpg", "file")).not.toBe(fileIconMarkup("report.pdf", "file"));
  });

  it("lets a whole name decide before its extension does", () => {
    expect(fileIconMarkup("Dockerfile", "file")).not.toBe(fileIconMarkup("notes.txt", "file"));
    expect(fileIconMarkup("readme.md", "file")).not.toBe(fileIconMarkup("notes.md", "file"));
    expect(fileIconMarkup("package-lock.json", "file")).toBe(fileIconMarkup("package.json", "file"));
  });

  it("keeps the generic glyph for a type it does not know, a link and a device", () => {
    expect(fileIconMarkup("notes.xyz", "file")).toBeNull();
    expect(fileIconMarkup("shortcut", "symlink")).toBeNull();
    expect(fileIconMarkup("sda", "other")).toBeNull();
  });

  it("leaves a directory to the component's solid folder and an application bundle to a program", () => {
    expect(fileIconMarkup("logs", "directory"), "the folder is drawn by the component, not this table").toBeNull();
    expect(fileIconMarkup("Safari.app", "directory")).toBe(fileIconMarkup("setup.exe", "file"));
  });

  it("treats a dotfile as configuration, unless its whole name says otherwise", () => {
    expect(fileIconMarkup(".bashrc", "file")).toBe(fileIconMarkup(".env", "file"));
    expect(fileIconMarkup(".gitignore", "file")).not.toBe(fileIconMarkup(".bashrc", "file"));
  });
});
