/**
 * What an external server's two forms are made of, as pure functions.
 *
 * The rules live here rather than inside `McpSettings.vue` because each of them is a decision the
 * form can get wrong silently — an argument split on spaces, a secret whose name was left blank —
 * and a decision that is testable without mounting a page is one that can be held still.
 */

/** One name/value row of the secrets editor. Both boxes are empty until somebody types. */
export interface KeyValueDraft {
  key: string;
  value: string;
}

/**
 * The arguments a stdio server is started with, one per line.
 *
 * One per line rather than one space-separated line: an argument may contain a space, and a form
 * that split on spaces would quietly hand the server two arguments where somebody typed one.
 */
export function parseServerArgs(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line !== "");
}

/**
 * The record a set of rows stands for, or `null` when the rows cannot become one.
 *
 * Rows with both boxes empty are what a fresh editor holds and are skipped; a value with no name has
 * nowhere to go and two rows with one name would keep only the last one, so both are refused rather
 * than resolved by picking a winner. Keys are trimmed — the name is a key, not a value — and values
 * are left exactly as typed, because a secret's surrounding whitespace is part of the secret.
 */
export function keyValueRecord(rows: readonly KeyValueDraft[]): Record<string, string> | null {
  const out: Record<string, string> = {};
  const seen = new Set<string>();
  for (const row of rows) {
    const key = row.key.trim();
    if (key === "") {
      if (row.value !== "") {
        return null;
      }
      continue;
    }
    if (seen.has(key)) {
      return null;
    }
    seen.add(key);
    out[key] = row.value;
  }
  return out;
}
