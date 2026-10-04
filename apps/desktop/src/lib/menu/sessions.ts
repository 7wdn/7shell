/**
 * What can be done to a saved session, to a group in the list, and to a forward that is up.
 *
 * Everything here is a button the sidebar already has, moved to where a person looks for it. Deleting
 * still asks its question in the row rather than here: the confirmation belongs next to the name it
 * is about, and there is exactly one of them.
 *
 * The address is the one thing worth copying, and it is copied the way the sidebar prints it — with
 * the user and the port — because that is the string somebody pastes into another tool.
 *
 * A group's menu never offers anything about the sessions inside it. Renaming, moving and deleting a
 * group are all changes to the group; what is inside stays where it is, which is also what the vault
 * does when a group goes away.
 */
import { ArrowDown, ArrowUp, Copy, Folder, FolderInput, FolderPlus, Pencil, Terminal, Trash2, X } from "@lucide/vue";

import { drawOrder, groupPath } from "@/lib/sessions/groups";
import type { OpenTunnel, Session, SessionGroup } from "@/types/session";

import { copyText } from "./clipboard";
import type { MenuEntry, MenuLeaf } from "./types";

export interface SessionMenuActions {
  readonly openTerminal: (session: Session) => void;
  readonly openFiles: (session: Session) => void;
  readonly edit: (session: Session) => void;
  readonly remove: (session: Session) => void;
  /** Into a group, or out of every group when the id is absent. */
  readonly move: (session: Session, groupId: string | undefined) => void;
}

/** The address as the sidebar writes it, which is what somebody would type somewhere else. */
export function sessionEndpoint(session: Session): string {
  return `${session.user}@${session.host}:${session.port}`;
}

export function sessionMenu(
  session: Session,
  groups: readonly SessionGroup[],
  actions: SessionMenuActions,
): MenuEntry[] {
  // A session whose group was deleted reads as one that is in no group, which is where it sits.
  const inGroup = groups.some((group) => group.id === session.groupId) ? session.groupId : undefined;

  // Drawn in the tree's own order rather than the order the vault happened to list them in, and
  // named by their path: a menu that puts the groups in a different order than the list they sit
  // in is a menu people misread, and two groups may share a name at different depths.
  const moveEntries: MenuLeaf[] = drawOrder(groups).map(({ group }) => ({
    kind: "action",
    id: `move-to-${group.id}`,
    // The entry names a group somebody made, so the label is the name itself: there is no sentence
    // to translate, and the key is the placeholder that says so.
    labelKey: "sessions.list.groupLabel",
    params: { name: groupPath(groups, group.id) },
    checked: inGroup === group.id,
    run: () => actions.move(session, group.id),
  }));
  if (moveEntries.length > 0) {
    moveEntries.unshift({
      kind: "action",
      id: "move-to-ungrouped",
      labelKey: "sessions.editor.groupNone",
      checked: inGroup === undefined,
      run: () => actions.move(session, undefined),
    });
  }

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
    ...(moveEntries.length > 0
      ? ([
          {
            kind: "submenu",
            id: "move-to-group",
            labelKey: "menu.moveToGroup",
            icon: FolderInput,
            entries: moveEntries,
          },
        ] satisfies MenuEntry[])
      : []),
    { kind: "separator", id: "sep-remove" },
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

export interface GroupMenuActions {
  readonly rename: (group: SessionGroup) => void;
  readonly remove: (group: SessionGroup) => void;
  readonly move: (group: SessionGroup, direction: -1 | 1) => void;
  /** Makes a group inside this one, which is how nesting is made without dragging. */
  readonly addInside: (group: SessionGroup) => void;
}

/**
 * What can be done to a group heading.
 *
 * The two move entries are drawn at the ends of their own parent's list but not runnable there: a
 * list where the first heading offers "Move up" and nothing happens is a list that looks broken,
 * and there is no fifth thing a person could mean.
 */
export function groupMenu(
  group: SessionGroup,
  actions: GroupMenuActions,
  ends: { readonly up: boolean; readonly down: boolean },
): MenuEntry[] {
  return [
    {
      kind: "action",
      id: "rename-group",
      labelKey: "sessions.list.groupRename",
      icon: Pencil,
      run: () => actions.rename(group),
    },
    {
      kind: "action",
      id: "add-group-inside",
      labelKey: "sessions.list.groupNewInside",
      icon: FolderPlus,
      run: () => actions.addInside(group),
    },
    { kind: "separator", id: "sep-move-group" },
    {
      kind: "action",
      id: "move-group-up",
      labelKey: "sessions.list.groupUp",
      icon: ArrowUp,
      disabled: !ends.up,
      run: () => actions.move(group, -1),
    },
    {
      kind: "action",
      id: "move-group-down",
      labelKey: "sessions.list.groupDown",
      icon: ArrowDown,
      disabled: !ends.down,
      run: () => actions.move(group, 1),
    },
    { kind: "separator", id: "sep-remove-group" },
    {
      kind: "action",
      id: "remove-group",
      labelKey: "sessions.list.groupRemove",
      icon: Trash2,
      danger: true,
      run: () => actions.remove(group),
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
