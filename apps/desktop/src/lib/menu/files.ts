/**
 * What can be done to the rows somebody has selected, and to the directory they are listed in.
 *
 * Every entry here is an action the panel already had as a button — a menu that could do something no
 * button could would be a second way to change a host, and the panel is where those decisions are
 * argued about. What is new is that the list is a *selection* rather than one row, so the entries are
 * counted: opening, editing and renaming are one thing at a time (they always were), while deleting,
 * copying and downloading are the whole selection at once.
 *
 * Three things are stated where they happen rather than left to be discovered:
 *
 * - **A folder is downloaded packed or not at all**, so the entry for it only exists while that switch
 *   is on — and once several rows are selected, the same entry takes them all as one archive.
 * - **The unpacked way of taking several rows needs all of them to be files**, because a directory
 *   cannot come down file by file: there would be nothing to ask for.
 * - **The upload entry says "unpack" when that is what it will do.** A menu that kept the same words
 *   while a row did something else is a menu nobody can trust.
 *
 * Deleting asks for confirmation in the panel's own strip rather than here, and that is on purpose:
 * the question — "delete this, and everything under it?" — is worth a line of its own, and one place
 * to answer it is one place to get it right.
 */
import {
  ClipboardPaste,
  Copy,
  Download,
  FilePlus,
  FileText,
  FolderOpen,
  FolderPlus,
  Pencil,
  Plus,
  RefreshCw,
  Trash2,
  Upload,
} from "@lucide/vue";

import { copyText } from "./clipboard";
import type { MenuEntry } from "./types";
import type { SftpEntry } from "@/types/session";

export interface FileEntryActions {
  readonly open: (entry: SftpEntry) => void;
  /**
   * Takes the whole selection, one way or the other.
   *
   * Whether several rows become one archive or one transfer each is decided where the dialogs are —
   * that is where the question "which folder" can be asked — so the menu says which of the two it was
   * offering and the panel passes it on.
   */
  readonly download: (entries: readonly SftpEntry[], packed: boolean) => void;
  readonly edit: (entry: SftpEntry) => void;
  readonly rename: (entry: SftpEntry) => void;
  readonly remove: (entries: readonly SftpEntry[]) => void;
}

export interface FileMenuOptions {
  /** Whether transfers travel as archives: the switch in the directory's own menu and the settings. */
  readonly packed: boolean;
}

/** Whether this entry can be read as a file at all, which is not true of a socket or a device. */
function readable(entry: SftpEntry): boolean {
  return entry.kind === "file" || entry.kind === "symlink";
}

export function fileEntryMenu(
  entries: readonly SftpEntry[],
  actions: FileEntryActions,
  options: FileMenuOptions,
): MenuEntry[] {
  const only = entries.length === 1 ? entries[0] : undefined;
  const count = String(entries.length);
  const many = entries.length > 1;
  const chosen: MenuEntry[] = [];

  // One directory, opened: a double-click's worth of meaning. Opening several at once is not
  // something this panel can do — it shows one directory at a time.
  if (only?.kind === "directory") {
    chosen.push({
      kind: "action",
      id: "open",
      labelKey: "files.action.open",
      icon: FolderOpen,
      run: () => actions.open(only),
    });
  }

  // The packed way takes anything: files, directories, or a mixture — that is what an archive is.
  if (options.packed && (many || only?.kind === "directory")) {
    chosen.push({
      kind: "action",
      id: "download-archive",
      labelKey: many ? "menu.downloadItems" : "menu.downloadFolder",
      params: many ? { count } : undefined,
      icon: Download,
      run: () => actions.download(entries, true),
    });
  }

  // The plain way: one file at a time — which for several files means one folder to put them in.
  //
  // A single file is copied as it always was, whatever the switch says: there is nothing to pack, and
  // a switch that took the download away from one file would be a switch that broke the common case.
  // Several rows have no plain way while packing is on, because the archive is the faster one; and a
  // directory has none at all, since there is nothing to ask for file by file.
  const plainWay = entries.length > 0 && entries.every(readable) && (!options.packed || entries.length === 1);
  if (plainWay) {
    chosen.push({
      kind: "action",
      id: "download",
      labelKey: many ? "menu.downloadSeveral" : "files.action.download",
      params: many ? { count } : undefined,
      icon: Download,
      run: () => actions.download(entries, false),
    });
  }

  if (only !== undefined && readable(only)) {
    chosen.push({
      kind: "action",
      id: "edit",
      labelKey: "files.action.edit",
      icon: FileText,
      run: () => actions.edit(only),
    });
  }

  if (chosen.length > 0) {
    chosen.push({ kind: "separator", id: "sep-manage" });
  }

  // One name can be changed at a time: renaming five rows would need five names typed, and the panel
  // has one rename field.
  if (only !== undefined) {
    chosen.push({
      kind: "action",
      id: "rename",
      labelKey: "files.action.rename",
      icon: Pencil,
      run: () => actions.rename(only),
    });
  }

  chosen.push(
    {
      kind: "action",
      id: "remove",
      labelKey: many ? "menu.deleteItems" : "files.action.remove",
      params: many ? { count } : undefined,
      icon: Trash2,
      danger: true,
      run: () => actions.remove(entries),
    },
    { kind: "separator", id: "sep-copy" },
    {
      kind: "action",
      id: "copy-name",
      labelKey: many ? "menu.copyNames" : "menu.copyName",
      params: many ? { count } : undefined,
      icon: Copy,
      run: () => void copyText(entries.map((entry) => entry.name).join("\n")),
    },
    {
      kind: "action",
      id: "copy-path",
      labelKey: many ? "menu.copyPaths" : "menu.copyPath",
      params: many ? { count } : undefined,
      icon: Copy,
      run: () => void copyText(entries.map((entry) => entry.path).join("\n")),
    },
  );

  const target = only?.kind === "symlink" ? only.linkTarget : undefined;
  if (target !== undefined && target !== "") {
    chosen.push({
      kind: "action",
      id: "copy-target",
      labelKey: "menu.copyTarget",
      icon: Copy,
      run: () => void copyText(target),
    });
  }

  return chosen;
}

export interface FileAreaState {
  /** The directory on screen, which is what every entry here acts on. */
  readonly path: string;
  /** Whether transfers are packed, which is what the switch in this menu is set to. */
  readonly packed: boolean;
}

export interface FileAreaActions {
  readonly newFolder: () => void;
  readonly newFile: () => void;
  readonly upload: () => void;
  /** Uploads whatever files the clipboard is holding, which is what Ctrl+V does here. */
  readonly paste: () => void;
  readonly refresh: () => void;
  readonly togglePacked: () => void;
}

export function fileAreaMenu(state: FileAreaState, actions: FileAreaActions): MenuEntry[] {
  return [
    { kind: "action", id: "refresh", labelKey: "menu.refresh", icon: RefreshCw, run: () => actions.refresh() },
    { kind: "separator", id: "sep-transfer" },
    {
      kind: "action",
      id: "upload",
      // The same entry does something else once packing is on, so it says something else: several
      // files are chosen and packed here, rather than one file being copied across.
      labelKey: state.packed ? "menu.packedUpload" : "files.upload",
      icon: Upload,
      run: () => actions.upload(),
    },
    {
      kind: "action",
      id: "paste",
      labelKey: "files.paste.label",
      icon: ClipboardPaste,
      // The one accelerator worth printing anywhere in this menu: the shell really does register it,
      // and only while a browser is on screen — which is exactly when this menu can be opened. It is
      // written as a binding rather than as a sentence, so the platform it is drawn on decides
      // whether that reads `Ctrl + V` or `⌘ V`.
      accel: "Mod+V",
      run: () => actions.paste(),
    },
    {
      kind: "action",
      id: "packed",
      labelKey: "menu.packedTransfer",
      checked: state.packed,
      run: () => actions.togglePacked(),
    },
    { kind: "separator", id: "sep-new" },
    {
      kind: "submenu",
      id: "new",
      labelKey: "menu.new",
      icon: Plus,
      entries: [
        {
          kind: "action",
          id: "new-folder",
          labelKey: "files.newFolder",
          icon: FolderPlus,
          run: () => actions.newFolder(),
        },
        { kind: "action", id: "new-file", labelKey: "files.newFile", icon: FilePlus, run: () => actions.newFile() },
      ],
    },
    { kind: "separator", id: "sep-copy" },
    {
      kind: "action",
      id: "copy-path",
      labelKey: "menu.copyPath",
      icon: Copy,
      disabled: state.path === "",
      run: () => void copyText(state.path),
    },
  ];
}
