/**
 * The directory bookmark rules.
 *
 * The popover only asks "can this draft be saved" and "what does the list look like now", so the
 * rules are asserted here rather than through a mounted component: the interesting failures are a
 * name that collides in another case, and a cap that must not block renaming.
 */
import { describe, expect, it } from "vitest";

import {
  FILE_BOOKMARK_LABEL_MAX,
  FILE_BOOKMARK_LIMIT,
  addFileBookmark,
  bookmarkLabelForPath,
  removeFileBookmark,
  sanitizeFileBookmarks,
  updateFileBookmark,
  validateFileBookmark,
} from "@/lib/files/bookmarks";

const logs = { id: "b1", label: "logs", path: "/var/log" };
const conf = { id: "b2", label: "nginx conf", path: "/etc/nginx" };

describe("bookmarkLabelForPath", () => {
  it("starts a new bookmark with the last segment of its path", () => {
    expect(bookmarkLabelForPath("/usr/local/nginx/conf")).toBe("conf");
  });

  it("ignores a trailing separator", () => {
    expect(bookmarkLabelForPath("/var/log/")).toBe("log");
  });

  it("falls back to the root when there is no segment to take", () => {
    expect(bookmarkLabelForPath("/")).toBe("/");
    expect(bookmarkLabelForPath("   ")).toBe("/");
  });
});

describe("validateFileBookmark", () => {
  it("accepts a draft that breaks none of the rules", () => {
    expect(validateFileBookmark({ label: "sites", path: "/srv/www" }, [logs])).toBeNull();
  });

  it("names the first thing wrong, in the order somebody would fix it", () => {
    expect(validateFileBookmark({ label: "  ", path: "" }, [])).toBe("labelEmpty");
    expect(validateFileBookmark({ label: "l".repeat(FILE_BOOKMARK_LABEL_MAX + 1), path: "/" }, [])).toBe(
      "labelTooLong",
    );
    expect(validateFileBookmark({ label: "sites", path: "   " }, [])).toBe("pathEmpty");
    expect(validateFileBookmark({ label: "sites", path: `/${"x".repeat(1024)}` }, [])).toBe("pathTooLong");
  });

  it("refuses a name another bookmark already has, in any case", () => {
    expect(validateFileBookmark({ label: "LOGS", path: "/srv" }, [logs])).toBe("labelDuplicate");
  });

  it("allows an entry to keep its own name while being edited", () => {
    expect(validateFileBookmark({ label: "logs", path: "/var/log/messages" }, [logs], { id: "b1" })).toBeNull();
  });

  it("refuses a new entry once the list is full, while letting an existing one be renamed", () => {
    const full = Array.from({ length: FILE_BOOKMARK_LIMIT }, (_, index) => ({
      id: `b${index}`,
      label: `folder ${index}`,
      path: `/srv/${index}`,
    }));

    expect(validateFileBookmark({ label: "one more", path: "/srv" }, full)).toBe("limitReached");
    expect(validateFileBookmark({ label: "renamed", path: "/srv/0" }, full, { id: "b0" })).toBeNull();
  });
});

describe("the list after a change", () => {
  it("keeps entries in the order the popover draws, regardless of when they were added", () => {
    const list = addFileBookmark([logs], { label: "apache", path: "/etc/apache2" }, "b3");

    expect(list.map((bookmark) => bookmark.label)).toEqual(["apache", "logs"]);
  });

  it("writes the trimmed values somebody meant to type", () => {
    const list = addFileBookmark([], { label: "  sites  ", path: "  /srv/www  " }, "b3");

    expect(list).toEqual([{ id: "b3", label: "sites", path: "/srv/www" }]);
  });

  it("updates one entry in place and sorts it by its new name", () => {
    const list = updateFileBookmark([logs, conf], "b1", { label: "zzz logs", path: "/var/log" });

    expect(list.map((bookmark) => bookmark.label)).toEqual(["nginx conf", "zzz logs"]);
    expect(list[1]).toEqual({ id: "b1", label: "zzz logs", path: "/var/log" });
  });

  it("removes only the entry named", () => {
    expect(removeFileBookmark([logs, conf], "b1")).toEqual([conf]);
    expect(removeFileBookmark([logs, conf], "missing")).toEqual([logs, conf]);
  });
});

describe("sanitizeFileBookmarks", () => {
  it("keeps what the popover can draw and drops the rest", () => {
    const list = sanitizeFileBookmarks([
      { id: " a ", label: " logs ", path: " /var/log " },
      { id: "b", label: "  ", path: "/tmp" },
      { id: "c", label: "Logs", path: "/var/log/nginx" },
      { id: "a", label: "other", path: "/srv" },
      { label: "no id", path: "/x" },
      "not an entry",
    ]);

    expect(list).toEqual([{ id: "a", label: "logs", path: "/var/log" }]);
  });

  it("answers with an empty list for a value that is not a list", () => {
    expect(sanitizeFileBookmarks(undefined)).toEqual([]);
    expect(sanitizeFileBookmarks({ bookmarks: [] })).toEqual([]);
  });

  it("keeps no more than the cap", () => {
    const rows = Array.from({ length: FILE_BOOKMARK_LIMIT + 3 }, (_, index) => ({
      id: `b${index}`,
      label: `folder ${index}`,
      path: `/srv/${index}`,
    }));

    expect(sanitizeFileBookmarks(rows)).toHaveLength(FILE_BOOKMARK_LIMIT);
  });
});
