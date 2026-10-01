/**
 * What can be done to a saved session, and to a forward that is up.
 *
 * Everything here is a button the sidebar already has, moved to where a person looks for it. Deleting
 * still asks its question in the row rather than here: the confirmation belongs next to the name it
 * is about, and there is exactly one of them.
 *
 * The address is the one thing worth copying, and it is copied the way the sidebar prints it — with
 * the user and the port — because that is the string somebody pastes into another tool.
 */
import { Copy, Folder, Pencil, Terminal, Trash2, X } from "@lucide/vue";

import type { OpenTunnel, Session } from "@/types/session";

import { copyText } from "./clipboard";
import type { MenuEntry } from "./types";

export interface SessionMenuActions {
  readonly openTerminal: (session: Session) => void;
  readonly openFiles: (session: Session) => void;
  readonly edit: (session: Session) => void;
  readonly remove: (session: Session) => void;
}

/** The address as the sidebar writes it, which is what somebody would type somewhere else. */
export function sessionEndpoint(session: Session): string {
  return `${session.user}@${session.host}:${session.port}`;
}

export function sessionMenu(session: Session, actions: SessionMenuActions): MenuEntry[] {
  return [
    {
      kind: "action",
      id: "open-terminal",
      labelKey: "sessions.list.open",
      params: { name: session.name },
      icon: Terminal,
      run: () => actions.openTerminal(session),
    },
    {
      kind: "action",
      id: "open-files",
      labelKey: "files.open",
      params: { name: session.name },
      icon: Folder,
      run: () => actions.openFiles(session),
    },
    { kind: "separator", id: "sep-edit" },
    { kind: "action", id: "edit", labelKey: "sessions.list.edit", icon: Pencil, run: () => actions.edit(session) },
    {
      kind: "action",
      id: "remove",
      labelKey: "sessions.list.remove",
      icon: Trash2,
      danger: true,
      run: () => actions.remove(session),
    },
    { kind: "separator", id: "sep-copy" },
    {
      kind: "action",
      id: "copy-endpoint",
      labelKey: "menu.copyAddress",
      icon: Copy,
      run: () => void copyText(sessionEndpoint(session)),
    },
  ];
}

export interface TunnelMenuActions {
  readonly stop: (id: string) => void;
}

export function tunnelMenu(tunnel: OpenTunnel, actions: TunnelMenuActions): MenuEntry[] {
  const forward = `${tunnel.bindHost}:${tunnel.bindPort} → ${tunnel.targetHost}:${tunnel.targetPort}`;
  return [
    {
      kind: "action",
      id: "stop",
      labelKey: "terminal.tunnels.stop",
      icon: X,
      danger: true,
      run: () => actions.stop(tunnel.id),
    },
    { kind: "separator", id: "sep-copy" },
    { kind: "action", id: "copy-forward", labelKey: "menu.copyForward", icon: Copy, run: () => void copyText(forward) },
  ];
}
