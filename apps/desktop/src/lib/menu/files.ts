/**
 * What can be done to a remote file, a remote directory, and to the directory they are listed in.
 *
 * Every entry here is an action the panel already had as a button — a menu that could do something no
 * button could would be a second way to change a host, and the panel is where those decisions are
 * argued about. Nothing is offered that the panel refuses to offer either: a socket file has no Edit,
 * and a directory has no plain Download, because there is nothing to download it into.
 *
 * Two things in this file depend on the packed-transfer switch, and both are stated where they happen
 * rather than left to be discovered: a directory is offered a download at all only while that switch
 * is on, and the upload entry says "unpack" when that is what it will do. A menu that kept the same
 * words while a row would do something else is a menu nobody can trust.
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
  readonly download: (entry: SftpEntry) => void;
  /** Brings a directory down as one archive, which is the only way it can come down. */
  readonly downloadArchive: (entry: SftpEntry) => void;
  readonly edit: (entry: SftpEntry) => void;
  readonly rename: (entry: SftpEntry) => void;
  readonly remove: (entry: SftpEntry) => void;
}

export interface FileMenuOptions {
  /** Whether directories travel as archives: the switch in the directory's own menu and the settings. */
  readonly packed: boolean;
}

export function fileEntryMenu(entry: SftpEntry, actions: FileEntryActions, options: FileMenuOptions): MenuEntry[] {
  // The same two rules the row's own buttons follow: files and links are downloaded and edited,
  // and only a directory is opened.
  const readable = entry.kind === "file" || entry.kind === "symlink";
  const entries: MenuEntry[] = [];

  if (entry.kind === "directory") {
    entries.push({
      kind: "action",
      id: "open",
      labelKey: "files.action.open",
      icon: FolderOpen,
      run: () => actions.open(entry),
    });
    if (options.packed) {
      entries.push({
        kind: "action",
        id: "download-archive",
        labelKey: "menu.downloadFolder",
        icon: Download,
        run: () => actions.downloadArchive(entry),
      });
    }
  }
  if (readable) {
    entries.push(
      {
        kind: "action",
        id: "download",
        labelKey: "files.action.download",
        icon: Download,
        run: () => actions.download(entry),
      },
      {
        kind: "action",
        id: "edit",
        labelKey: "files.action.edit",
        icon: FileText,
        run: () => actions.edit(entry),
      },
    );
  }
  if (entries.length > 0) {
    entries.push({ kind: "separator", id: "sep-manage" });
  }

  entries.push(
    {
      kind: "action",
      id: "rename",
      labelKey: "files.action.rename",
      icon: Pencil,
      run: () => actions.rename(entry),
    },
    {
      kind: "action",
      id: "remove",
      labelKey: "files.action.remove",
      icon: Trash2,
      danger: true,
      run: () => actions.remove(entry),
    },
    { kind: "separator", id: "sep-copy" },
    { kind: "action", id: "copy-name", labelKey: "menu.copyName", icon: Copy, run: () => void copyText(entry.name) },
    { kind: "action", id: "copy-path", labelKey: "menu.copyPath", icon: Copy, run: () => void copyText(entry.path) },
  );

  const target = entry.linkTarget;
  if (entry.kind === "symlink" && target !== undefined && target !== "") {
    entries.push({
      kind: "action",
      id: "copy-target",
      labelKey: "menu.copyTarget",
      icon: Copy,
      run: () => void copyText(target),
    });
  }

  return entries;
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
      // The same entry does something else once packing is on, so it says something else: an archive
      // is chosen and unpacked, rather than a file being copied across.
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
      // and only while a browser is on screen — which is exactly when this menu can be opened.
      accel: "Ctrl+V",
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
