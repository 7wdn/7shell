/**
 * What can be done to a transfer, which depends on where it is.
 *
 * One entry plus the copy, and the entry is whatever the row would need next: hold a running
 * transfer, let a paused one go, try a stopped one again, dismiss a finished one. A transfer that
 * was *cancelled* is dismissed rather than cancelled — calling it "Cancel" would promise something
 * that is no longer possible — and a failed one offers the retry rather than the dismissal, because
 * a person looking at a failure wants it to work, not to go away.
 *
 * The remote path is the only part of a transfer the interface ever held — the local end was chosen
 * in a native dialog and never came back this way — so it is also the only thing worth copying.
 */
import { Copy, Play, RotateCcw, X } from "@lucide/vue";

import type { TransferInfo } from "@/types/session";

import { copyText } from "./clipboard";
import type { MenuEntry } from "./types";

export interface TransferMenuActions {
  readonly cancel: (id: string) => void;
  readonly setPaused: (id: string, paused: boolean) => void;
  readonly retry: (id: string) => void;
}

export function transferMenu(transfer: TransferInfo, actions: TransferMenuActions): MenuEntry[] {
  return [
    ...primaryEntry(transfer, actions),
    { kind: "separator", id: "sep-copy" },
    {
      kind: "action",
      id: "copy-path",
      labelKey: "menu.copyPath",
      icon: Copy,
      run: () => void copyText(transfer.remotePath),
    },
  ];
}

function primaryEntry(transfer: TransferInfo, actions: TransferMenuActions): MenuEntry[] {
  switch (transfer.state.kind) {
    case "running":
      return [
        {
          kind: "action",
          id: "pause",
          labelKey: "files.transfer.pause",
          run: () => actions.setPaused(transfer.id, true),
        },
        {
          kind: "action",
          id: "cancel",
          labelKey: "files.transfer.cancel",
          icon: X,
          run: () => actions.cancel(transfer.id),
        },
      ];
    case "paused":
      return [
        {
          kind: "action",
          id: "resume",
          labelKey: "files.transfer.resume",
          icon: Play,
          run: () => actions.setPaused(transfer.id, false),
        },
        {
          kind: "action",
          id: "cancel",
          labelKey: "files.transfer.cancel",
          icon: X,
          run: () => actions.cancel(transfer.id),
        },
      ];
    case "failed":
      return [
        {
          kind: "action",
          id: "retry",
          labelKey: "files.transfer.retry",
          icon: RotateCcw,
          run: () => actions.retry(transfer.id),
        },
        {
          kind: "action",
          id: "dismiss",
          labelKey: "files.transfer.dismiss",
          icon: X,
          run: () => actions.cancel(transfer.id),
        },
      ];
    default:
      return [
        {
          kind: "action",
          id: "dismiss",
          labelKey: "files.transfer.dismiss",
          run: () => actions.cancel(transfer.id),
        },
      ];
  }
}
