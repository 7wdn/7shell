/**
 * Says the one sentence a lost transfer deserves, when the list that would have shown it was not on
 * screen.
 *
 * The transfer list is drawn inside a file browser, so a transfer that fails while the person is in
 * a terminal leaves no trace they would find on their own: nothing is drawn, nothing is counted, and
 * the row waits in a panel they have no reason to open. This is the "action whose context is gone"
 * case the toast exists for.
 *
 * When a file browser *is* on screen, nothing is said: the list is already the notice, a failed row
 * that stays until it is dismissed, and one message must not have two homes. Either way the failure
 * is consumed here — whether it was visible was a fact about that moment, not about the failure.
 */
import { watch } from "vue";
import { useI18n } from "vue-i18n";

import { useToast } from "@/composables/useToast";
import { useFilesStore } from "@/stores/files";
import { useTerminalsStore } from "@/stores/terminals";
import type { TransferDirection } from "@/types/session";

export function useTransferNotices(): void {
  const { t } = useI18n();
  const files = useFilesStore();
  const terminals = useTerminalsStore();
  const { toast } = useToast();

  function directionLabel(direction: TransferDirection): string {
    return direction === "upload" ? t("files.transfer.direction.upload") : t("files.transfer.direction.download");
  }

  /** Every file browser shows the same list, so any of them can serve as the way back to it. */
  function showTransfers(): void {
    const target = [...terminals.tabs].reverse().find((entry) => entry.kind === "files");
    if (target) {
      terminals.activate(target.id);
    }
  }

  watch(
    () => files.unattendedFailures,
    (waiting) => {
      if (waiting.length === 0) {
        return;
      }
      files.attendFailures(waiting.map((entry) => entry.id));

      const listIsOnScreen = terminals.activeTab?.kind === "files";
      if (listIsOnScreen) {
        return;
      }

      const only = waiting.length === 1 ? waiting[0] : undefined;
      const message = only
        ? t("files.transfer.failedAway", { direction: directionLabel(only.direction), name: only.name })
        : t("files.transfer.failedAwayCount", { count: waiting.length });

      const reachable = terminals.tabs.some((entry) => entry.kind === "files");
      toast(
        message,
        "unseenFailure",
        reachable ? { label: t("files.transfer.showList"), onClick: showTransfers } : undefined,
      );
    },
  );
}
