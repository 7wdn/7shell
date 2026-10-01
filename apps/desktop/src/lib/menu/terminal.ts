/**
 * What can be done to a terminal from its own right-click menu.
 *
 * Copy and paste are the reason this menu exists — they are the two things a person reaches for in a
 * terminal and the two this application had no way of offering — and both are shaped by what the
 * terminal actually holds rather than by what the clipboard is expected to have.
 *
 * The selection is read when the menu is built, not when Copy is pressed, so an entry that cannot do
 * anything is drawn as one that cannot do anything. Details matter more than a theoretical race:
 * built when the menu opens, `Copy` is greyed out on a terminal with nothing selected, which is the
 * answer somebody is looking for.
 *
 * No shortcut is advertised. Ctrl+C in a terminal is SIGINT and Ctrl+V is the browser's paste into a
 * hidden field: printing either next to a menu entry would be teaching the wrong keys.
 */
import { ClipboardPaste, Copy, Eraser, SquareDashedMousePointer } from "@lucide/vue";

import { copyText, readPastedText } from "./clipboard";
import type { MenuEntry } from "./types";

export interface TerminalMenuState {
  /** What is selected right now, which is what Copy would take. */
  readonly selection: string;
  /** A terminal that has ended takes no keystrokes, so it takes no paste either. */
  readonly interactive: boolean;
}

export interface TerminalMenuActions {
  /** Hands the text to the terminal, which is what a real paste would have done with it. */
  readonly paste: (text: string) => void;
  readonly selectAll: () => void;
  /** Throws the scrollback away, keeping the line the prompt is on. */
  readonly clear: () => void;
}

export function terminalMenu(state: TerminalMenuState, actions: TerminalMenuActions): MenuEntry[] {
  return [
    {
      kind: "action",
      id: "copy",
      labelKey: "menu.copy",
      icon: Copy,
      disabled: state.selection === "",
      run: () => void copyText(state.selection),
    },
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
