/**
 * What can be done to a tab in the workspace strip.
 *
 * Closing the others and closing the ones to the right are the two things a strip of tabs is always
 * missing, and both are offered as words rather than as a modifier key: this strip has no place to
 * print a shortcut, and a menu that is only reachable with the mouse should not need one.
 *
 * Reconnecting is offered only where the interface offers it elsewhere. A browser that already has a
 * handle is not a failed browser — it is an open one — and opening it again would be a second browser
 * on one session, which is the thing the panel's own retry button is careful not to do.
 */
import { PanelRightClose, RotateCw, SquareX, X } from "@lucide/vue";

import type { MenuEntry } from "./types";

export interface TabMenuState {
  /** Which half of the workspace this is: neither page has a stream to reconnect. */
  readonly kind: "terminal" | "files" | "inspect" | "settings" | "batch";
  /** `null` for a page, which has no stream of its own to have ended. */
  readonly status: "open" | "ended" | "failed" | null;
  /** Whether it is already drawing a browser: one with a handle must not be opened a second time. */
  readonly hasHandle: boolean;
  /** Where it sits in the strip, and how many there are — what "the others" and "the right" mean. */
  readonly index: number;
  readonly total: number;
}

export interface TabMenuActions {
  readonly close: () => void;
  readonly closeOthers: () => void;
  readonly closeToTheRight: () => void;
  readonly reconnect: () => void;
}

/** Whether this tab is one the interface would offer a retry for. */
export function canReconnect(state: TabMenuState): boolean {
  if (state.kind === "inspect" || state.kind === "settings" || state.kind === "batch") {
    return false;
  }
  if (state.kind === "files") {
    return !state.hasHandle;
  }
  return state.status !== "open";
}

export function tabMenu(state: TabMenuState, actions: TabMenuActions): MenuEntry[] {
  const others = state.total - 1;
  return [
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
      id: "close-right",
      labelKey: "menu.closeRight",
      icon: PanelRightClose,
      disabled: state.index >= others,
      run: () => actions.closeToTheRight(),
    },
    { kind: "separator", id: "sep-reconnect" },
    {
      kind: "action",
      id: "reconnect",
      labelKey: "menu.reconnect",
      icon: RotateCw,
      disabled: !canReconnect(state),
      run: () => actions.reconnect(),
    },
  ];
}
