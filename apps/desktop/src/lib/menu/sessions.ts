/**
 * What can be done to a saved session, to a group in the list, and to a forward that is up.
 *
 * Everything here is a button the sidebar already has, moved to where a person looks for it. Deleting
 * still asks its question in the row rather than here: the confirmation belongs next to the name it
 * is about, and there is exactly one of them.
 *
 * The address is the one thing worth copying, and it is copied the way the sidebar prints it — with
 * the user and the port — because that is the string somebody pastes into another tool. A group's
 * menu copies its name, which is the only one of its fields anybody has a use for elsewhere.
 *
 * A group's menu offers nothing about the *saved* sessions inside it. Renaming, moving and deleting a
 * group are all changes to the group; what is inside stays where it is, which is also what the vault
 * does when a group goes away. Closing what is open inside it is the one exception, and not really
 * one: it is about the connections this run has, not about the library — the count says how many
 * there are, and the sessions themselves are left alone.
 */
import {
  ArrowDown,
  ArrowUp,
  Copy,
  Folder,
  FolderInput,
  FolderPlus,
  Pencil,
  Pin,
  PinOff,
  Terminal,
  Trash2,
  Unplug,
  X,
} from "@lucide/vue";

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
  /**
   * Makes a group and puts this session in it, which is the way in for a library that has no groups
   * yet: without it, "move into a group" is a thing you can only do twice, and the second time is
   * the one you actually wanted.
   */
  readonly newGroup: (session: Session) => void;
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
    // At the end of the list of groups, where a list that is short one group continues: the entry
    // is a way of arriving at a group that is not there yet, and it reads as the last line of the
    // list it belongs to.
    moveEntries.push(
      { kind: "separator", id: "sep-new-group" },
      {
        kind: "action",
        id: "move-to-new-group",
        labelKey: "sessions.list.groupNewForSession",
        icon: FolderPlus,
        run: () => actions.newGroup(session),
      },
    );
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
      : // With no group to move into, the same thing is a row of its own — and it says what it does
        // to the session, because there is no list of destinations above it to read it against.
        ([
          {
            kind: "action",
            id: "move-to-new-group",
            labelKey: "sessions.list.moveToNewGroup",
            icon: FolderPlus,
            run: () => actions.newGroup(session),
          },
        ] satisfies MenuEntry[])),
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
  /**
   * Holds the group above the ones it was made beside, or lets it go again.
   *
   * The flag travels rather than being turned over inside the menu: the menu draws what is true now,
   * and what is written is the store's business.
   */
  readonly pin: (group: SessionGroup, pinned: boolean) => void;
  /** Closes every connection that is open inside it, subgroups included. */
  readonly close: (group: SessionGroup) => void;
}

/** What the heading's own state says about the entries that cannot always run. */
export interface GroupMenuState {
  /** Whether there is a group to move past on that side, within the band it is drawn in. */
  readonly up: boolean;
  readonly down: boolean;
  /** How many connections are open inside it right now; nothing to close when it is zero. */
  readonly open: number;
}

/**
 * What can be done to a group heading.
 *
 * The two move entries are drawn at the ends of their own parent's list but not runnable there: a
 * list where the first heading offers "Move up" and nothing happens is a list that looks broken,
 * and there is no fifth thing a person could mean. Closing is drawn the same way — the count is in
 * its own label, and with nothing open inside there is nothing for it to do.
 */
export function groupMenu(group: SessionGroup, actions: GroupMenuActions, state: GroupMenuState): MenuEntry[] {
  return [
    {
      kind: "action",
      id: group.pinned ? "unpin-group" : "pin-group",
      labelKey: group.pinned ? "sessions.list.groupUnpin" : "sessions.list.groupPin",
      icon: group.pinned ? PinOff : Pin,
      run: () => actions.pin(group, !group.pinned),
    },
    { kind: "separator", id: "sep-pin-group" },
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
    {
      kind: "action",
      id: "close-group-connections",
      labelKey: "sessions.list.groupClose",
      params: { count: String(state.open) },
      icon: Unplug,
      disabled: state.open === 0,
      run: () => actions.close(group),
    },
    { kind: "separator", id: "sep-move-group" },
    {
      kind: "action",
      id: "move-group-up",
      labelKey: "sessions.list.groupUp",
      icon: ArrowUp,
      disabled: !state.up,
      run: () => actions.move(group, -1),
    },
    {
      kind: "action",
      id: "move-group-down",
      labelKey: "sessions.list.groupDown",
      icon: ArrowDown,
      disabled: !state.down,
      run: () => actions.move(group, 1),
    },
    { kind: "separator", id: "sep-copy-group" },
    {
      kind: "action",
      id: "copy-group-name",
      labelKey: "menu.copyName",
      icon: Copy,
      run: () => void copyText(group.name),
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
