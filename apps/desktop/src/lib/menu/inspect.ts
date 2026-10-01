/**
 * What can be done on the two host pages: the process table and the socket tables.
 *
 * Both are read-only views of a host, and their menus stay that way. What a row can do is be read
 * more closely — a process opened under the table, a port's connections shown below it — or have one
 * of its fields copied out, which is the reason these pages exist at all: somebody has found the
 * process that is eating the machine and wants its command line somewhere they can use it.
 *
 * An entry that copies a field the host did not report is greyed out rather than offered and then
 * refused. A kernel thread has no executable path, and copying an empty string would be a lie told
 * quietly.
 */
import { Copy, Eye, RefreshCw } from "@lucide/vue";

import type { EnvironmentEntry, ProcessDetail, ProcessSample, SocketRow } from "@/types/monitor";

import { copyText } from "./clipboard";
import type { MenuEntry } from "./types";

export interface ProcessRowActions {
  readonly open: (pid: number, name: string) => void;
}

export function processRowMenu(process: ProcessSample, actions: ProcessRowActions): MenuEntry[] {
  const executable = process.executable ?? "";
  return [
    {
      kind: "action",
      id: "open-detail",
      labelKey: "menu.openDetail",
      icon: Eye,
      run: () => actions.open(process.pid, process.name),
    },
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
    {
      kind: "action",
      id: "copy-command",
      labelKey: "menu.copyCommand",
      icon: Copy,
      disabled: process.command === "",
      run: () => void copyText(process.command),
    },
    {
      kind: "action",
      id: "copy-executable",
      labelKey: "menu.copyExecutable",
      icon: Copy,
      disabled: executable === "",
      run: () => void copyText(executable),
    },
  ];
}

export interface ProcessDetailActions {
  readonly refresh: () => void;
}

export function processDetailMenu(detail: ProcessDetail, actions: ProcessDetailActions): MenuEntry[] {
  const copy = (id: string, labelKey: string, value: string): MenuEntry => ({
    kind: "action",
    id,
    labelKey,
    icon: Copy,
    disabled: value === "",
    run: () => void copyText(value),
  });

  return [
    copy("copy-command", "menu.copyCommand", detail.commandLine),
    copy("copy-executable", "menu.copyExecutable", detail.executable),
    copy("copy-working-directory", "menu.copyWorkingDirectory", detail.workingDirectory),
    { kind: "separator", id: "sep-refresh" },
    {
      kind: "action",
      id: "refresh",
      labelKey: "inspect.process.refresh",
      icon: RefreshCw,
      run: () => actions.refresh(),
    },
  ];
}

export function environmentMenu(entry: EnvironmentEntry): MenuEntry[] {
  const pair = `${entry.name}=${entry.value}`;
  return [
    { kind: "action", id: "copy-value", labelKey: "menu.copyValue", icon: Copy, run: () => void copyText(entry.value) },
    { kind: "action", id: "copy-pair", labelKey: "menu.copyVariable", icon: Copy, run: () => void copyText(pair) },
  ];
}

export interface ListenerActions {
  /** Shows the connections of this socket under the table, which is what clicking the row does. */
  readonly showConnections: (socket: SocketRow) => void;
}

export function listenerMenu(socket: SocketRow, actions: ListenerActions): MenuEntry[] {
  const endpoint = `${socket.address}:${socket.port}`;
  return [
    {
      kind: "action",
      id: "show-connections",
      labelKey: "menu.viewConnections",
      icon: Eye,
      run: () => actions.showConnections(socket),
    },
    { kind: "separator", id: "sep-copy" },
    {
      kind: "action",
      id: "copy-address",
      labelKey: "menu.copyAddress",
      icon: Copy,
      run: () => void copyText(socket.address),
    },
    {
      kind: "action",
      id: "copy-port",
      labelKey: "menu.copyPort",
      icon: Copy,
      run: () => void copyText(String(socket.port)),
    },
    {
      kind: "action",
      id: "copy-endpoint",
      labelKey: "menu.copyEndpoint",
      icon: Copy,
      run: () => void copyText(endpoint),
    },
    {
      kind: "action",
      id: "copy-process",
      labelKey: "menu.copyProcess",
      icon: Copy,
      disabled: socket.process === "",
      run: () => void copyText(socket.process),
    },
    {
      kind: "action",
      id: "copy-pid",
      labelKey: "menu.copyPid",
      icon: Copy,
      // Zero is the host's way of saying it would not name the process holding the socket.
      disabled: socket.pid === 0,
      run: () => void copyText(String(socket.pid)),
    },
  ];
}

export function connectionMenu(connection: SocketRow): MenuEntry[] {
  return [
    {
      kind: "action",
      id: "copy-peer-address",
      labelKey: "menu.copyPeerAddress",
      icon: Copy,
      run: () => void copyText(connection.peerAddress),
    },
    {
      kind: "action",
      id: "copy-peer-port",
      labelKey: "menu.copyPeerPort",
      icon: Copy,
      run: () => void copyText(String(connection.peerPort)),
    },
    {
      kind: "action",
      id: "copy-state",
      labelKey: "menu.copyState",
      icon: Copy,
      disabled: connection.state === "",
      run: () => void copyText(connection.state),
    },
  ];
}
