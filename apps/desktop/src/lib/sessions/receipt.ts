/**
 * What an import did, said as a receipt.
 *
 * One implementation because two places start an import — the sidebar's own menu and the home
 * page's shortcuts — and a second copy of these sentences is exactly how the two would come to
 * describe the same file differently.
 *
 * The counts are joined rather than fixed into one sentence because what comes out of a file is not
 * known in advance: an empty import, a plain one, and one that carried a warning all have to read
 * as the same kind of sentence. Anything that could not be carried is named here or nowhere — the
 * session is in the list either way, and this is the only moment anybody can be told why it might
 * not connect the way it did where the file came from.
 */

import type { Composer } from "vue-i18n";

import type { SessionImportSummary } from "@/types/session";

/** The interface's own translation, as something with no component of its own can take it. */
export type ReceiptTranslate = Composer["t"];

export function importReceipt(summary: SessionImportSummary, t: ReceiptTranslate): string {
  const parts: string[] = [];
  const counted = (key: string, count: number): string => t(key, { count }, count);
  if (summary.sessions > 0) {
    parts.push(counted("sessions.list.importSessions", summary.sessions));
  }
  if (summary.groups > 0) {
    parts.push(counted("sessions.list.importGroups", summary.groups));
  }
  if (summary.secrets > 0) {
    parts.push(counted("sessions.list.importSecrets", summary.secrets));
  }
  if (summary.missingKeys > 0) {
    parts.push(counted("sessions.list.importMissingKeys", summary.missingKeys));
  }
  if (summary.droppedHops > 0) {
    parts.push(counted("sessions.list.importDroppedHops", summary.droppedHops));
  }
  return parts.length === 0
    ? t("sessions.list.importNothing")
    : t("sessions.list.importDone", { parts: parts.join(", ") });
}
