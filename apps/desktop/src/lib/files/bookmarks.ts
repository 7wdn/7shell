/**
 * Directory bookmarks: the named remote paths the file browser offers as quick jumps.
 *
 * The rules live here rather than in the popover because they are about the data, not the drawing:
 * one name per list (in any case), a cap the list can be read at a glance, and a path that is not
 * checked for existence — a bookmark may point at a mount that is not there today. Everything in
 * this file is a value in and a value out, so each rule is asserted without a component.
 */

import type { FileBookmark } from "@/types/app";

export const FILE_BOOKMARK_LIMIT = 20;
export const FILE_BOOKMARK_LABEL_MAX = 64;
export const FILE_BOOKMARK_PATH_MAX = 1024;

/**
 * Why a draft cannot be saved, in the order the checks run. The popover maps each to a sentence;
 * the first failure is the one worth fixing.
 */
export type FileBookmarkError =
  | "labelEmpty"
  | "labelTooLong"
  | "pathEmpty"
  | "pathTooLong"
  | "limitReached"
  | "labelDuplicate";

/** A draft on its way to the list: the two values somebody can type. */
export interface FileBookmarkInput {
  readonly label: string;
  readonly path: string;
}

/** Length as the file's reader counts it — characters, not UTF-16 units — so both sides agree. */
function charCount(value: string): number {
  return [...value].length;
}

/** The name a new bookmark starts with: the last segment of the path it points at. */
export function bookmarkLabelForPath(path: string): string {
  const segments = path.trim().split("/").filter(Boolean);
  const last = segments[segments.length - 1] || "/";
  return [...last].slice(0, FILE_BOOKMARK_LABEL_MAX).join("");
}

/** The order the popover draws: by name, in any case, with numbers read as numbers. */
export function sortFileBookmarks(bookmarks: readonly FileBookmark[]): FileBookmark[] {
  return [...bookmarks].sort((left, right) =>
    left.label.localeCompare(right.label, undefined, { numeric: true, sensitivity: "base" }),
  );
}

/**
 * Checks a draft against the rules, in the order a person would notice them.
 *
 * `id` names the entry being edited, if any: it is skipped by the name check and by the cap, since
 * renaming a bookmark at the limit is not making another one.
 */
export function validateFileBookmark(
  input: FileBookmarkInput,
  existing: readonly FileBookmark[],
  options: { readonly id?: string; readonly limit?: number } = {},
): FileBookmarkError | null {
  const label = input.label.trim();
  if (!label) return "labelEmpty";
  if (charCount(label) > FILE_BOOKMARK_LABEL_MAX) return "labelTooLong";
  const path = input.path.trim();
  if (!path) return "pathEmpty";
  if (charCount(path) > FILE_BOOKMARK_PATH_MAX) return "pathTooLong";
  const editing = options.id !== undefined && existing.some((bookmark) => bookmark.id === options.id);
  if (!editing && existing.length >= (options.limit ?? FILE_BOOKMARK_LIMIT)) return "limitReached";
  const lowered = label.toLowerCase();
  if (existing.some((bookmark) => bookmark.id !== options.id && bookmark.label.toLowerCase() === lowered)) {
    return "labelDuplicate";
  }
  return null;
}

/** Adds an entry that has already passed `validateFileBookmark`, keeping the list sorted. */
export function addFileBookmark(list: readonly FileBookmark[], input: FileBookmarkInput, id: string): FileBookmark[] {
  return sortFileBookmarks([...list, { id, label: input.label.trim(), path: input.path.trim() }]).slice(
    0,
    FILE_BOOKMARK_LIMIT,
  );
}

/** Rewrites one entry's two values, keeping its place in the sorted order. */
export function updateFileBookmark(
  list: readonly FileBookmark[],
  id: string,
  input: FileBookmarkInput,
): FileBookmark[] {
  return sortFileBookmarks(
    list.map((bookmark) =>
      bookmark.id === id ? { ...bookmark, label: input.label.trim(), path: input.path.trim() } : bookmark,
    ),
  );
}

/** Removes one entry by id; an id that is not there leaves the list as it was. */
export function removeFileBookmark(list: readonly FileBookmark[], id: string): FileBookmark[] {
  return list.filter((bookmark) => bookmark.id !== id);
}

/**
 * The list as the popover can draw it.
 *
 * A settings file is a boundary like any other: an entry without a name or a path is a row that
 * goes nowhere, two sharing a name read as a duplicate of itself, and entries past the cap could
 * not be added through the popover. The first of each id and each name wins, so a hand-edited file
 * cannot make which entry a click reaches depend on file order.
 */
export function sanitizeFileBookmarks(raw: unknown): FileBookmark[] {
  if (!Array.isArray(raw)) return [];
  const seenIds = new Set<string>();
  const seenLabels = new Set<string>();
  const out: FileBookmark[] = [];
  for (const row of raw) {
    if (!row || typeof row !== "object" || Array.isArray(row)) continue;
    const record = row as Record<string, unknown>;
    const id = typeof record.id === "string" ? record.id.trim() : "";
    const label = typeof record.label === "string" ? record.label.trim() : "";
    const path = typeof record.path === "string" ? record.path.trim() : "";
    if (!id || !label || !path) continue;
    if (charCount(label) > FILE_BOOKMARK_LABEL_MAX || charCount(path) > FILE_BOOKMARK_PATH_MAX) continue;
    const lowered = label.toLowerCase();
    if (seenIds.has(id) || seenLabels.has(lowered)) continue;
    seenIds.add(id);
    seenLabels.add(lowered);
    out.push({ id, label, path });
    if (out.length >= FILE_BOOKMARK_LIMIT) break;
  }
  return out;
}
