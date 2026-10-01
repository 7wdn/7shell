/**
 * What a terminal's right-click offers.
 *
 * The two rules worth pinning down are the ones a screenshot cannot show: Copy is offered only when
 * there is something selected, and Paste only into a terminal that still takes keystrokes.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import { copyText, readPastedText } from "./clipboard";
import { terminalMenu } from "./terminal";
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
  return { paste: vi.fn(), selectAll: vi.fn(), clear: vi.fn() };
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

  it("clears the screen and selects everything through the terminal", () => {
    const wired = actions();
    const entries = terminalMenu({ selection: "", interactive: true }, wired);

    pick(entries, "select-all").run();
    pick(entries, "clear").run();

    expect(wired.selectAll).toHaveBeenCalledOnce();
    expect(wired.clear).toHaveBeenCalledOnce();
  });
});
