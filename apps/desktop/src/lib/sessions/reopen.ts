/**
 * A session that is already open, and what opening it again means.
 *
 * Two questions, and they are the same situation: what the session list's menu does when the session
 * it names is already open, and what a second tab about one session is called. Both are pure
 * functions over what the store already knows — how many tabs are open for the session, and the
 * order the tabs were opened in — so the answers can be read here rather than inferred from a store
 * that is halfway through opening something.
 *
 * The remembered answer is read by the menu path only. Clicking a session row is the plain open it
 * has always been, and nothing here is consulted for it.
 */
import type { SessionReopen } from "@/types/app";

/** What opening a session does, once the tabs it already has are known. */
export type ReopenAction =
  /** Open it: either the first tab, or a second one a remembered answer asked for. */
  | "open"
  /** Bring the tab that is already open forward. */
  | "focus"
  /** Ask about a second tab — the sidebar is where the question is drawn. */
  | "ask";

/**
 * What the menu's open does with a session that may already be open.
 *
 * A session with nothing open is simply opened, whatever the remembered answer says: the question is
 * about a *second* tab, and asking it before the first one exists would be asking about a tab that
 * is not there.
 */
export function reopenAction(openTabs: number, remembered: SessionReopen): ReopenAction {
  if (openTabs === 0) {
    return "open";
  }
  if (remembered === "new-tab") {
    return "open";
  }
  return remembered === "focus" ? "focus" : "ask";
}

/** One tab as the numbering below reads it. */
export interface SessionTabLike {
  readonly id: string;
  readonly sessionId: string;
  /** What kind of a tab it is: a terminal and its file manager are not copies of each other. */
  readonly kind: "terminal" | "files";
}

/**
 * Which copy of its session each tab is, by tab id: 0 for the first, 1 for the second, and so on.
 *
 * The order is the one the tabs were *opened* in, not the order they are drawn in: a tab somebody
 * dragged is still the same tab, and a sort must not renumber anybody. Numbers collapse when a tab
 * is closed — the tab after the one that went away becomes the first of what is left — which is why
 * this is read off the living tabs rather than written down when a tab is made.
 *
 * Counted per session and per kind, because a file manager standing beside a terminal is not a
 * second terminal: numbering the pair would call the browser "lab (1)" and imply somebody had
 * opened the same thing twice.
 */
export function sessionCopyIndex(tabs: readonly SessionTabLike[], opened: readonly string[]): Record<string, number> {
  const byId = new Map(tabs.map((tab) => [tab.id, tab]));
  const counted = new Map<string, number>();
  const copies: Record<string, number> = {};

  for (const id of opened) {
    const tab = byId.get(id);
    if (!tab) {
      continue;
    }
    // The session and the kind as one key rather than a map of maps: the kind is one of two words
    // and can hold no separator, so the pair is a string nobody has to nest for.
    const key = `${tab.sessionId}\u0000${tab.kind}`;
    const copy = counted.get(key) ?? 0;
    copies[id] = copy;
    counted.set(key, copy + 1);
  }

  return copies;
}

/**
 * What a tab about a session is called.
 *
 * The session's own name is the first tab's, because that is the tab somebody means when they say
 * the name; every copy after it is numbered from one, so the second terminal about `lab` is
 * `lab (1)`. Numbering starts at one rather than at zero because there is no "lab (0)": the tab
 * without a number *is* the zero.
 */
export function sessionTabTitle(name: string, copy: number): string {
  return copy === 0 ? name : `${name} (${copy})`;
}
