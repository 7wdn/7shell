/**
 * What can be done to a terminal from its own right-click menu.
 *
 * Copy and paste are the reason this menu exists — they are the two things a person reaches for in a
 * terminal and the two this application had no way of offering — and both are shaped by what the
 * terminal actually holds rather than by what the clipboard is expected to have.
 *
 * Paste selection is the third: what is highlighted goes to the shell as if it had been pasted,
 * without the clipboard and without a copy in between. It is the shortest way from something the
 * output said to a command line that uses it, and the toolbar drawn over a selection offers it
 * beside Copy — `terminalSelectionTools` below builds that half, and both surfaces take the same
 * entries so that one action cannot mean two things in two places.
 *
 * The selection is read when the menu is built, not when Copy is pressed, so an entry that cannot do
 * anything is drawn as one that cannot do anything. Details matter more than a theoretical race:
 * built when the menu opens, `Copy` is greyed out on a terminal with nothing selected, which is the
 * answer somebody is looking for.
 *
 * No shortcut is advertised. Ctrl+C in a terminal is SIGINT and Ctrl+V is the browser's paste into a
 * hidden field: printing either next to a menu entry would be teaching the wrong keys. The one
 * exception is find, whose key this application really does listen for.
 */
import { ClipboardPaste, ClipboardType, Copy, Eraser, Search, SquareDashedMousePointer } from "@lucide/vue";

import { copyText, readPastedText } from "./clipboard";
import type { MenuAction, MenuEntry } from "./types";

export interface TerminalMenuState {
  /** What is selected right now, which is what Copy and Paste selection both take. */
  readonly selection: string;
  /** A terminal that has ended takes no keystrokes, so it takes no paste either. */
  readonly interactive: boolean;
}

/** Handing the selection to the shell, which is done the same way wherever it is offered. */
interface SelectionHandoff {
  /** Hands the text to the terminal, which is what a real paste would have done with it. */
  readonly paste: (text: string) => void;
  /** Lets the highlight go, once what it held has been handed over. */
  readonly dropSelection: () => void;
}

export interface TerminalMenuActions extends SelectionHandoff {
  readonly selectAll: () => void;
  /** Throws the scrollback away, keeping the line the prompt is on. */
  readonly clear: () => void;
}

/** What the toolbar over a selection can do: a smaller set than the menu's. */
export interface TerminalSelectionActions extends SelectionHandoff {
  /**
   * Opens the pane's find bar over the text it is given.
   *
   * The text and not only the bar: what the magnifier is pressed *about* is the highlight under it,
   * and a bar that opened empty would be asking somebody to write down by hand what they had just
   * selected.
   */
  readonly find: (needle: string) => void;
}

function copyEntry(state: TerminalMenuState): MenuAction {
  return {
    kind: "action",
    id: "copy",
    labelKey: "menu.copy",
    icon: Copy,
    disabled: state.selection === "",
    run: () => void copyText(state.selection),
  };
}

/**
 * What is highlighted goes to the shell, and the highlight goes with it.
 *
 * Dropped rather than left standing: the selection has been handed over, and a highlight that stayed
 * would invite a second press — which would type the same text into the same prompt again.
 */
function pasteSelectionEntry(state: TerminalMenuState, actions: SelectionHandoff): MenuAction {
  return {
    kind: "action",
    id: "paste-selection",
    labelKey: "menu.pasteSelection",
    icon: ClipboardType,
    disabled: state.selection === "" || !state.interactive,
    run: () => {
      actions.paste(state.selection);
      actions.dropSelection();
    },
  };
}

function findEntry(state: TerminalMenuState, actions: { readonly find: (needle: string) => void }): MenuAction {
  return {
    kind: "action",
    id: "find",
    labelKey: "terminal.search.label",
    icon: Search,
    // The one entry in either surface that may print a key, because the terminal really does keep
    // this one from the shell — see the note about shortcuts at the top.
    shortcut: "terminal.find",
    run: () => actions.find(state.selection),
  };
}

export function terminalMenu(state: TerminalMenuState, actions: TerminalMenuActions): MenuEntry[] {
  return [
    copyEntry(state),
    {
      kind: "action",
      id: "paste",
      labelKey: "menu.paste",
      icon: ClipboardPaste,
      disabled: !state.interactive,
      run: async () => {
        const text = await readPastedText();
        if (text !== null) {
          actions.paste(text);
        }
      },
    },
    pasteSelectionEntry(state, actions),
    {
      kind: "action",
      id: "select-all",
      labelKey: "menu.selectAll",
      icon: SquareDashedMousePointer,
      run: () => actions.selectAll(),
    },
    { kind: "separator", id: "sep-screen" },
    { kind: "action", id: "clear", labelKey: "menu.clear", icon: Eraser, run: () => actions.clear() },
  ];
}

/**
 * The little toolbar that appears over a selection: copy, hand it to the shell, find.
 *
 * The three things a person reaches for while their hand is still on the selection. Reaching for the
 * right-click menu to do them is a trip for nothing when the text is already highlighted.
 */
export function terminalSelectionTools(state: TerminalMenuState, actions: TerminalSelectionActions): MenuAction[] {
  return [copyEntry(state), pasteSelectionEntry(state, actions), findEntry(state, actions)];
}
