/**
 * What can be done to a tab in the workspace strip.
 *
 * A terminal's menu is about its connection as much as about its tab: a duplicate is one more
 * connection to the same machine, reconnecting is the same thing again, and disconnecting leaves the
 * tab where it is so the place can be reconnected. A browser and the pages have no connection of
 * their own, so those entries are not offered there at all rather than offered disabled.
 *
 * Closing the others, closing the ones to the right and closing everything are words rather than
 * modifier keys: this strip has no place to print a shortcut, and a menu that is only reachable with
 * the mouse should not need one.
 */
import { CircleX, Copy, PanelLeftClose, PanelRightClose, Plug, Power, RotateCw, SquareX, X } from "@lucide/vue";

import type { MenuEntry } from "./types";

export interface TabMenuState {
  /** Which half of the workspace this is: neither page has a stream to reconnect. */
  readonly kind: "terminal" | "files" | "inspect" | "settings" | "batch" | "container" | "kubernetes" | "vmware";
  /** `null` for a page, which has no stream of its own to have ended. */
  readonly status: "open" | "ended" | "failed" | null;
  /** Whether it is already drawing a browser: one with a handle must not be opened a second time. */
  readonly hasHandle: boolean;
  /** Where it sits in the strip, and how many there are — what "the others" and "the right" mean. */
  readonly index: number;
  readonly total: number;
  /** How many terminal tabs in the strip are not open, which is what "connect all" would reach. */
  readonly reconnectable: number;
}

export interface TabMenuActions {
  readonly duplicate: () => void;
  readonly close: () => void;
  readonly closeOthers: () => void;
  readonly closeToTheLeft: () => void;
  readonly closeToTheRight: () => void;
  readonly closeAll: () => void;
  readonly reconnect: () => void;
  readonly reconnectAll: () => void;
  readonly disconnect: () => void;
}

/** Whether this tab is one the interface would offer a retry for. */
export function canReconnect(state: TabMenuState): boolean {
  if (state.kind === "inspect" || state.kind === "settings" || state.kind === "batch") {
    return false;
  }
  if (state.kind === "files") {
    return !state.hasHandle;
  }
  if (state.kind === "container" || state.kind === "kubernetes") {
    return false;
  }
  return state.status !== "open";
}

export function tabMenu(state: TabMenuState, actions: TabMenuActions): MenuEntry[] {
  const others = state.total - 1;
  const closing: MenuEntry[] = [
    { kind: "action", id: "close", labelKey: "common.action.close", icon: X, run: () => actions.close() },
    {
      kind: "action",
      id: "close-others",
      labelKey: "menu.closeOthers",
      icon: SquareX,
      disabled: others === 0,
      run: () => actions.closeOthers(),
    },
    {
      kind: "action",
      id: "close-left",
      labelKey: "menu.closeLeft",
      icon: PanelLeftClose,
      disabled: state.index === 0,
      run: () => actions.closeToTheLeft(),
    },
    {
      kind: "action",
      id: "close-right",
      labelKey: "menu.closeRight",
      icon: PanelRightClose,
      disabled: state.index >= others,
      run: () => actions.closeToTheRight(),
    },
    {
      kind: "action",
      id: "close-all",
      labelKey: "menu.closeAll",
      icon: CircleX,
      run: () => actions.closeAll(),
    },
  ];

  if (state.kind === "terminal") {
    return [
      {
        kind: "action",
        id: "duplicate",
        labelKey: "menu.duplicateTab",
        icon: Copy,
        run: () => actions.duplicate(),
      },
      { kind: "separator", id: "sep-connection" },
      {
        kind: "action",
        id: "reconnect",
        labelKey: "menu.reconnect",
        icon: RotateCw,
        disabled: !canReconnect(state),
        run: () => actions.reconnect(),
      },
      {
        kind: "action",
        id: "reconnect-all",
        labelKey: "menu.connectAll",
        icon: Plug,
        disabled: state.reconnectable === 0,
        run: () => actions.reconnectAll(),
      },
      {
        kind: "action",
        id: "disconnect",
        labelKey: "menu.disconnect",
        icon: Power,
        disabled: state.status !== "open",
        run: () => actions.disconnect(),
      },
      { kind: "separator", id: "sep-close" },
      ...closing,
    ];
  }

  if (state.kind === "files") {
    return [
      {
        kind: "action",
        id: "reconnect",
        labelKey: "menu.reconnect",
        icon: RotateCw,
        disabled: !canReconnect(state),
        run: () => actions.reconnect(),
      },
      { kind: "separator", id: "sep-close" },
      ...closing,
    ];
  }

  return closing;
}
