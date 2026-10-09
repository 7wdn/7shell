/**
 * What a backup did, said as a receipt.
 *
 * The counts are joined rather than fixed into one sentence, for the reason the import receipt
 * gives: what a file holds is not known in advance, and an empty backup, a plain one and one that
 * carried a warning all have to read as the same kind of sentence. Anything the file could not
 * bring is named here or nowhere — the session is in the library either way, and this is the only
 * moment anybody can be told why it might not connect the way it did where the file came from.
 */

import type { Composer } from "vue-i18n";

import type { SyncApplySummary, SyncContents } from "@/types/sync";

/** The interface's own translation, as something with no component of its own can take it. */
export type SyncTranslate = Composer["t"];

/** The counts a sentence can be built from, which is also what a catalog holds. */
function partsOf(contents: SyncContents, t: SyncTranslate): string[] {
  const parts: string[] = [];
  const counted = (key: string, count: number): string => t(`sync.${key}`, { count }, count);

  for (const [key, count] of [
    ["sessions", contents.sessions],
    ["groups", contents.groups],
    ["snippets", contents.snippets],
    ["words", contents.words],
    ["history", contents.history],
    ["hosts", contents.knownHosts],
    ["settings", contents.settings],
    ["secrets", contents.secrets],
    ["keys", contents.keys],
  ] as const) {
    if (count > 0) {
      parts.push(counted(key, count));
    }
  }
  return parts;
}

/** What was written, and under which of the two answers about credentials. */
export function backupReceipt(contents: SyncContents, sealed: boolean, fileName: string, t: SyncTranslate): string {
  const parts = partsOf(contents, t);
  const joined = parts.length === 0 ? t("sync.settings", { count: 0 }) : parts.join(", ");

  return t(sealed ? "sync.exported" : "sync.exportedPlain", { parts: joined, file: fileName });
}

/** What a restore wrote, and what it could not. */
export function restoreReceipt(summary: SyncApplySummary, t: SyncTranslate): string {
  const parts = partsOf(summary.contents, t);
  const counted = (key: string, count: number): string => t(`sync.${key}`, { count }, count);

  for (const [key, count] of [
    ["replaced", summary.replaced],
    ["copied", summary.copied],
    ["missingKeys", summary.missingKeys],
    ["droppedHops", summary.droppedHops],
  ] as const) {
    if (count > 0) {
      parts.push(counted(key, count));
    }
  }

  return parts.length === 0 ? t("sync.nothingToSay") : t("sync.restored", { parts: parts.join(", ") });
}
