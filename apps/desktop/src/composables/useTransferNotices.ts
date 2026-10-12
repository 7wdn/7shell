/**
 * Says the one sentence a lost transfer deserves, when the panel that would have shown it is put
 * away.
 *
 * A failed transfer leaves a row in the panel, and a pill is all that is left of the panel when
 * somebody has tucked it away — a pill that says how much is going on and nothing about what
 * happened. This is the "action whose context is gone" case the toast exists for, and its one action
 * is the way back: the row waiting behind the pill.
 *
 * While the panel is *open* nothing is said, because the row is already the notice: it stays until
 * it is dismissed, and one message must not have two homes. Either way the failure is consumed here —
 * whether it was visible was a fact about that moment, not about the failure.
 */
import { watch } from "vue";
import { useI18n } from "vue-i18n";

import { useToast } from "@/composables/useToast";
import { useFilesStore } from "@/stores/files";
import type { TransferDirection } from "@/types/session";

export function useTransferNotices(): void {
  const { t } = useI18n();
  const files = useFilesStore();
  const { toast } = useToast();

  function directionLabel(direction: TransferDirection): string {
    return direction === "upload" ? t("files.transfer.direction.upload") : t("files.transfer.direction.download");
  }

  watch(
    () => files.unattendedFailures,
    (waiting) => {
      if (waiting.length === 0) {
        return;
      }
      files.attendFailures(waiting.map((entry) => entry.id));

      if (files.panelExpanded) {
        return;
      }

      const only = waiting.length === 1 ? waiting[0] : undefined;
      const message = only
        ? t("files.transfer.failedAway", { direction: directionLabel(only.direction), name: only.name })
        : t("files.transfer.failedAwayCount", { count: waiting.length });

      toast(message, "unseenFailure", {
        label: t("files.transfer.showList"),
        onClick: () => {
          files.panelExpanded = true;
        },
      });
    },
  );
}
