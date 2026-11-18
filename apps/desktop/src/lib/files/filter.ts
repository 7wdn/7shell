/**
 * The listing narrowed to the names somebody typed.
 *
 * A directory arrives whole — the windowed rows need the full count to draw an honest scrollbar —
 * so narrowing it is arithmetic on what is already here, not another conversation with the server.
 * That is what makes the answer immediate while a name is still being typed.
 */
import type { SftpEntry } from "@/types/session";

/** Entries whose name contains the query, ignoring case. A blank query keeps the whole listing. */
export function filterByName(entries: readonly SftpEntry[], query: string): readonly SftpEntry[] {
  const needle = query.trim().toLocaleLowerCase();
  if (needle === "") {
    return entries;
  }
  return entries.filter((entry) => entry.name.toLocaleLowerCase().includes(needle));
}
