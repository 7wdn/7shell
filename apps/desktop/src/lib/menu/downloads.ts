/**
 * The menu behind the download folder: where downloads land, and where they used to.
 *
 * Four kinds of entry, and each answers a question somebody looking at that strip could have. The
 * remembered folders, newest first, with the one in force ticked — a list of names, because that is
 * what the interface is told, and the way to tell two of them apart is to open one. Then the two
 * ways out: choosing a folder that is not in the list, and going back to being asked every time.
 *
 * The copy is written here rather than at the call site for the reason the other menus' is: the
 * strip draws what this returns, and a test can ask for every label in every language from one place.
 */
import { Check, FolderOpen, FolderPlus, Hand } from "@lucide/vue";

import type { DownloadFolderView } from "@/types/session";

import type { MenuEntry } from "./types";

export interface DownloadFolderMenuActions {
  /** Makes the remembered folder at this position the one downloads land in. */
  readonly use: (index: number) => void;
  /** Asks for a folder that is not in the list. */
  readonly choose: () => void;
  /** Goes back to asking on every download. */
  readonly ask: () => void;
}

export function downloadFolderMenu(view: DownloadFolderView, actions: DownloadFolderMenuActions): MenuEntry[] {
  return [
    ...view.history.map(
      (name, index): MenuEntry => ({
        kind: "action",
        id: `folder-${index}`,
        labelKey: "files.transfer.folder.remembered",
        params: { name },
        // Tick and icon go by position rather than by name: two folders on this machine can share
        // their last segment, and a tick decided by the name would then land on both of them.
        icon: index === 0 && view.current !== null ? Check : FolderOpen,
        checked: index === 0 && view.current !== null,
        run: () => actions.use(index),
      }),
    ),
    { kind: "separator", id: "sep-download-folder" },
    {
      kind: "action",
      id: "choose",
      labelKey: "files.transfer.folder.choose",
      icon: FolderPlus,
      run: () => actions.choose(),
    },
    {
      kind: "action",
      id: "ask",
      labelKey: "files.transfer.folder.ask",
      icon: Hand,
      checked: view.current === null,
      run: () => actions.ask(),
    },
  ];
}
