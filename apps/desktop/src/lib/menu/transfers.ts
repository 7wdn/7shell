/**
 * What can be done to a transfer, which depends on where it is.
 *
 * Three groups, and the first is about the file rather than the movement: a transfer with a local end
 * can be opened once it has finished, and pointed at in the system's file manager whatever it did —
 * which is how the part file of one that stopped is found. Those entries are decided by
 * `hasLocalPath`, so a packed upload (whose archive is deleted once it has been read) offers neither
 * rather than offering one that would do nothing.
 *
 * Then whatever the row needs next: hold a running transfer, let a paused one go, try a stopped one
 * again. And then the two ways of being finished with it — stopping it, which leaves the row behind
 * saying it was cancelled, and removing it, which takes the row away and stops the bytes first if
 * they are still moving.
 *
 * The remote path is the only part of a transfer the interface ever held — the local end was chosen
 * in a native dialog and never came back this way — so it is also the only thing worth copying.
 */
import { Copy, ExternalLink, FolderOpen, Pause, Play, RotateCcw, Trash2, X } from "@lucide/vue";

import type { TransferInfo } from "@/types/session";

import { copyText } from "./clipboard";
import type { MenuEntry } from "./types";

export interface TransferMenuActions {
  /** Stops it, and leaves the row behind saying it was cancelled. */
  readonly stop: (id: string) => void;
  /** Takes the row away, stopping the transfer first if it is still moving. */
  readonly remove: (id: string) => void;
  readonly open: (id: string) => void;
  readonly reveal: (id: string) => void;
  readonly setPaused: (id: string, paused: boolean) => void;
  readonly retry: (id: string) => void;
}

export function transferMenu(transfer: TransferInfo, actions: TransferMenuActions): MenuEntry[] {
  return [
    ...localEndEntries(transfer, actions),
    ...primaryEntry(transfer, actions),
    ...removalEntries(transfer, actions),
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

/**
 * The two entries about the file itself.
 *
 * Opening waits for `done` — handing an application a half-written file is a trap, and a part file
 * opened as a document looks like a file that is merely short. Pointing at where it is does not
 * wait: that is exactly how somebody finds the part file of one that stopped.
 */
function localEndEntries(transfer: TransferInfo, actions: TransferMenuActions): MenuEntry[] {
  if (!transfer.hasLocalPath) {
    return [];
  }

  return [
    ...(transfer.state.kind === "done"
      ? [
          {
            kind: "action" as const,
            id: "open",
            labelKey: "files.transfer.open",
            icon: ExternalLink,
            run: () => actions.open(transfer.id),
          },
        ]
      : []),
    {
      kind: "action",
      id: "reveal",
      labelKey: "files.transfer.reveal",
      icon: FolderOpen,
      run: () => actions.reveal(transfer.id),
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
          icon: Pause,
          run: () => actions.setPaused(transfer.id, true),
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
      ];
    default:
      return [];
  }
}

/**
 * Stopping it, and being done with the row.
 *
 * Both, for a transfer that is still moving: stopping keeps the row so the outcome stays visible,
 * and removing is for somebody who has seen enough and wants it gone. For one that has stopped there
 * is nothing left to stop, so only the removal is offered — an entry promising to stop something
 * that already finished would be a lie the row itself contradicts.
 */
function removalEntries(transfer: TransferInfo, actions: TransferMenuActions): MenuEntry[] {
  const moving = transfer.state.kind === "running" || transfer.state.kind === "paused";

  return [
    ...(moving
      ? [
          {
            kind: "action" as const,
            id: "stop",
            labelKey: "files.transfer.cancel",
            icon: X,
            run: () => actions.stop(transfer.id),
          },
        ]
      : []),
    {
      kind: "action",
      id: "remove",
      labelKey: "files.transfer.remove",
      icon: Trash2,
      danger: true,
      run: () => actions.remove(transfer.id),
    },
  ];
}
