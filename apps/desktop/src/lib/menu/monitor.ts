/**
 * What can be done from the monitor panel's own small tables.
 *
 * The panel is a glance, not a workspace: the one thing it can do that the pages cannot do better is
 * hand a value to another window. So the entries are copies, and one way through to the page that
 * knows more — offered only where there is a page to open, which is a question the panel answers
 * before it builds the list.
 */
import { Copy, List } from "@lucide/vue";

import type { DiskUsage, ProcessSample } from "@/types/monitor";

import { copyText } from "./clipboard";
import type { MenuEntry } from "./types";

export interface MonitorProcessActions {
  readonly openPage: () => void;
}

export function monitorProcessMenu(process: ProcessSample, actions: MonitorProcessActions): MenuEntry[] {
  return [
    { kind: "action", id: "open-page", labelKey: "monitor.processes.title", icon: List, run: () => actions.openPage() },
    { kind: "separator", id: "sep-copy" },
    {
      kind: "action",
      id: "copy-pid",
      labelKey: "menu.copyPid",
      icon: Copy,
      run: () => void copyText(String(process.pid)),
    },
    {
      kind: "action",
      id: "copy-name",
      labelKey: "menu.copyProcess",
      icon: Copy,
      run: () => void copyText(process.name),
    },
  ];
}

export function diskMenu(disk: DiskUsage, actions: { readonly openPage: () => void }): MenuEntry[] {
  return [
    { kind: "action", id: "open-page", labelKey: "inspect.system.title", icon: List, run: () => actions.openPage() },
    { kind: "separator", id: "sep-copy" },
    {
      kind: "action",
      id: "copy-mount",
      labelKey: "menu.copyMountPoint",
      icon: Copy,
      run: () => void copyText(disk.mountPoint),
    },
    {
      kind: "action",
      id: "copy-filesystem",
      labelKey: "menu.copyFilesystem",
      icon: Copy,
      run: () => void copyText(disk.filesystem),
    },
  ];
}
