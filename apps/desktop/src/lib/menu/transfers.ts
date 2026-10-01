/**
 * What can be done to a transfer that is running, or to one that has finished.
 *
 * One entry, two words: the backend removes the record either way, and a transfer that was cancelled
 * half way through is dismissed rather than cancelled. Calling the second one "Cancel" would promise
 * something that is no longer possible.
 *
 * The remote path is the only part of a transfer the interface ever held — the local end was chosen
 * in a native dialog and never came back this way — so it is also the only thing worth copying.
 */
import { Copy, X } from "@lucide/vue";

import type { TransferInfo } from "@/types/session";

import { copyText } from "./clipboard";
import type { MenuEntry } from "./types";

export interface TransferMenuActions {
  readonly cancel: (id: string) => void;
}

export function transferMenu(transfer: TransferInfo, actions: TransferMenuActions): MenuEntry[] {
  const running = transfer.state.kind === "running";
  return [
    {
      kind: "action",
      id: "cancel",
      labelKey: running ? "files.transfer.cancel" : "files.transfer.dismiss",
      icon: running ? X : undefined,
      run: () => actions.cancel(transfer.id),
    },
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
