/**
 * What a context menu is made of.
 *
 * A menu is data rather than markup. The region that owns the thing under the pointer is the only
 * one that knows what can be done to it, so that region builds the list; one layer owned by the shell
 * draws it. Neither half needs to know the other: a file row, a process row and a tab hand over the
 * same shape.
 *
 * An entry carries a message key rather than a translated sentence, so a list can be built before it
 * is drawn and the language can change while it is open.
 */
import type { Component } from "vue";

import type { ShortcutActionId } from "@/lib/shortcuts/registry";

/** One thing somebody can do, or one thing they can see but not do. */
export interface MenuAction {
  readonly kind: "action";
  /** Stable within its list, so a test can name the entry it means. */
  readonly id: string;
  readonly labelKey: string;
  readonly params?: Readonly<Record<string, string>>;
  readonly icon?: Component;
  /**
   * The action whose key is shown on the right, or nothing.
   *
   * The action and not the key: a key is a string somebody can change in the settings, and the menu
   * has to print the one in force. It is only ever set where the key really works — a menu that
   * advertises a shortcut the application does not implement is worse than one that promises nothing.
   */
  readonly shortcut?: ShortcutActionId;
  /** Drawn, but not runnable. Why is the caller's business — a menu is no place for reasons. */
  readonly disabled?: boolean;
  /** Destructive: drawn in the colour of something that cannot be undone. */
  readonly danger?: boolean;
  /**
   * Present when the entry is a choice rather than an action.
   *
   * An entry with this set is drawn as a switch that is either on or off, and running it is expected
   * to turn it over — the menu does not tick it itself, because what the switch means belongs to
   * whoever put it there.
   */
  readonly checked?: boolean;
  readonly run: () => void | Promise<void>;
}

/** A line between two groups. Never focused, never run. */
export interface MenuSeparator {
  readonly kind: "separator";
  readonly id: string;
}

/** What a submenu may hold: things to run, and lines between them. */
export type MenuLeaf = MenuAction | MenuSeparator;

/**
 * A row that opens a list of its own beside the menu.
 *
 * It is not runnable: pressing it or pointing at it opens the list, and the list is where the
 * choices are. A submenu holds leaves only, so the depth is one — written here rather than enforced
 * at the drawing end, where a deeper list would simply not appear.
 */
export interface MenuSubmenu {
  readonly kind: "submenu";
  readonly id: string;
  readonly labelKey: string;
  readonly icon?: Component;
  readonly entries: readonly MenuLeaf[];
}

export type MenuEntry = MenuAction | MenuSeparator | MenuSubmenu;

/** Whether the keyboard can land on this entry. A submenu is a destination like any other row. */
export function canFocus(entry: MenuEntry): boolean {
  if (entry.kind === "separator") {
    return false;
  }
  return entry.kind === "submenu" || entry.disabled !== true;
}
