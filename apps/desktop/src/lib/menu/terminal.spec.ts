/**
 * What a terminal's right-click offers, and what the toolbar over a selection offers.
 *
 * The rules worth pinning down are the ones a screenshot cannot show: Copy is offered only when
 * there is something selected, Paste and Paste selection only into a terminal that still takes
 * keystrokes, and the hand-over of a selection is the same act from both surfaces.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText, readPastedText } from "./clipboard";
import { terminalMenu, terminalSelectionTools } from "./terminal";
import type { MenuAction, MenuEntry } from "./types";

vi.mock("./clipboard", () => ({
  copyText: vi.fn(async () => true),
  readPastedText: vi.fn(async () => "from the clipboard"),
}));

function pick(entries: readonly MenuEntry[], id: string): MenuAction {
  const found = entries.find((entry): entry is MenuAction => entry.kind === "action" && entry.id === id);
  if (!found) {
    throw new Error(`the menu has no entry called ${id}`);
  }
  return found;
}

function actions() {
  return { paste: vi.fn(), selectAll: vi.fn(), clear: vi.fn(), dropSelection: vi.fn() };
}

function toolbarActions() {
  return { paste: vi.fn(), dropSelection: vi.fn(), find: vi.fn() };
}

beforeEach(() => {
  vi.mocked(copyText).mockClear();
  vi.mocked(readPastedText).mockClear();
});

describe("the terminal's menu", () => {
  it("offers copy only when there is something selected", () => {
    const empty = terminalMenu({ selection: "", interactive: true }, actions());
    const selected = terminalMenu({ selection: "systemctl status", interactive: true }, actions());

    expect(pick(empty, "copy").disabled).toBe(true);
    expect(pick(selected, "copy").disabled).toBeFalsy();
  });

  it("copies what was selected", async () => {
    const entries = terminalMenu({ selection: "systemctl status", interactive: true }, actions());

    await pick(entries, "copy").run();

    expect(copyText).toHaveBeenCalledWith("systemctl status");
  });

  it("offers paste only into a terminal that still takes keystrokes", () => {
    const live = terminalMenu({ selection: "", interactive: true }, actions());
    const ended = terminalMenu({ selection: "", interactive: false }, actions());

    expect(pick(live, "paste").disabled).toBeFalsy();
    expect(pick(ended, "paste").disabled).toBe(true);
  });

  it("hands the clipboard's text to the terminal", async () => {
    const wired = actions();
    const entries = terminalMenu({ selection: "", interactive: true }, wired);

    await pick(entries, "paste").run();

    expect(wired.paste).toHaveBeenCalledWith("from the clipboard");
  });

  it("pastes nothing when the webview will not read the clipboard", async () => {
    vi.mocked(readPastedText).mockResolvedValueOnce(null);
    const wired = actions();
    const entries = terminalMenu({ selection: "", interactive: true }, wired);

    await pick(entries, "paste").run();

    // The refusal itself is said by the clipboard module; what matters here is that nothing was fed
    // to the shell on the strength of a clipboard nobody could read.
    expect(wired.paste).not.toHaveBeenCalled();
  });

  it("offers paste selection only with something selected, into a terminal that takes keystrokes", () => {
    const empty = terminalMenu({ selection: "", interactive: true }, actions());
    const ended = terminalMenu({ selection: "systemctl status", interactive: false }, actions());

    expect(pick(empty, "paste-selection").disabled).toBe(true);
    expect(pick(ended, "paste-selection").disabled).toBe(true);
  });

  it("hands the selection to the shell without the clipboard, and lets the highlight go", async () => {
    const wired = actions();
    const entries = terminalMenu({ selection: "systemctl status", interactive: true }, wired);

    await pick(entries, "paste-selection").run();

    expect(wired.paste).toHaveBeenCalledWith("systemctl status");
    expect(wired.dropSelection).toHaveBeenCalledOnce();
    expect(readPastedText, "the clipboard has no part in this").not.toHaveBeenCalled();
  });

  it("clears the screen and selects everything through the terminal", () => {
    const wired = actions();
    const entries = terminalMenu({ selection: "", interactive: true }, wired);

    pick(entries, "select-all").run();
    pick(entries, "clear").run();

    expect(wired.selectAll).toHaveBeenCalledOnce();
    expect(wired.clear).toHaveBeenCalledOnce();
  });
});

describe("the toolbar over a selection", () => {
  it("offers copy, paste selection and find, in that order", () => {
    const entries = terminalSelectionTools({ selection: "x", interactive: true }, toolbarActions());

    expect(entries.map((entry) => entry.id)).toEqual(["copy", "paste-selection", "find"]);
  });

  it("copies and hands over the same way the menu does", async () => {
    const wired = toolbarActions();
    const entries = terminalSelectionTools({ selection: "nginx -t", interactive: true }, wired);

    await pick(entries, "copy").run();
    await pick(entries, "paste-selection").run();

    expect(copyText).toHaveBeenCalledWith("nginx -t");
    expect(wired.paste).toHaveBeenCalledWith("nginx -t");
    expect(wired.dropSelection).toHaveBeenCalledOnce();
    expect(readPastedText, "the clipboard has no part in this either").not.toHaveBeenCalled();
  });

  it("keeps copy and paste selection away when they have nothing to act on", () => {
    const empty = terminalSelectionTools({ selection: "", interactive: true }, toolbarActions());
    const ended = terminalSelectionTools({ selection: "nginx -t", interactive: false }, toolbarActions());

    expect(pick(empty, "copy").disabled).toBe(true);
    expect(pick(empty, "paste-selection").disabled).toBe(true);
    expect(pick(ended, "paste-selection").disabled).toBe(true);
    // The scrollback of a terminal that has ended is still readable and still worth copying.
    expect(pick(ended, "copy").disabled).toBeFalsy();
  });

  it("advertises the find key, and no key for the other two", () => {
    const entries = terminalSelectionTools({ selection: "nginx -t", interactive: true }, toolbarActions());

    // Ctrl+C in a terminal is SIGINT, so no entry that copies or pastes may promise a key; find is
    // the one combination the terminal really does keep for the interface.
    expect(pick(entries, "find").shortcut).toBe("terminal.find");
    expect(pick(entries, "copy").shortcut).toBeUndefined();
    expect(pick(entries, "paste-selection").shortcut).toBeUndefined();
  });

  it("opens the find bar over what is selected", () => {
    const wired = toolbarActions();
    const entries = terminalSelectionTools({ selection: "nginx -t", interactive: true }, wired);

    pick(entries, "find").run();

    // The needle travels with the action: pressing the magnifier over a highlight is asking to look
    // for that highlight, not to open an empty bar.
    expect(wired.find).toHaveBeenCalledWith("nginx -t");
  });
});
