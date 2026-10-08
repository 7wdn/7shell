/**
 * The command line's own keys: what it offers while somebody types, what Tab takes, and what Enter
 * still means.
 *
 * The engine that decides the candidates has its own tests. This file is about the surface — that the
 * list appears for a word being typed, that the arrows move inside it without leaving the field, that
 * a row taken with Tab writes the line and leaves the caret after it, that Escape is obeyed until the
 * line changes, and that Enter goes on running the line somebody typed rather than one that is almost
 * it.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import CommandInputBar from "@/components/commands/CommandInputBar.vue";
import { i18n } from "@/i18n";
import { acceptanceBoost, forgetAccepted } from "@/lib/commands/acceptance";
import { useCommandsStore } from "@/stores/commands";
import { usePreferencesStore } from "@/stores/preferences";
import type { CommandHistoryEntry, Snippet } from "@/types/snippet";

let app: App | undefined;
let host: HTMLDivElement;

function snippet(name: string, command: string, prefix = ""): Snippet {
  return {
    id: name,
    groupId: "default",
    name,
    prefix,
    command,
    appendCr: true,
    enabled: true,
    order: 0,
    slotHistory: [],
  };
}

function ran(...commands: string[]): CommandHistoryEntry[] {
  return commands.map((command, at) => ({ command, at: 100 - at }));
}

function field(): HTMLInputElement {
  const input = host.querySelector<HTMLInputElement>("input");
  if (input === null) {
    throw new Error("the bar drew no input");
  }
  return input;
}

function rows(): HTMLElement[] {
  return [...host.querySelectorAll<HTMLElement>('[role="option"]')];
}

/** The row a screen reader would read out, which is where the keyboard is. */
function selected(): string | null {
  const id = field().getAttribute("aria-activedescendant");
  return id === null ? null : (host.querySelector<HTMLElement>(`#${id}`)?.textContent?.trim() ?? null);
}

async function type(text: string): Promise<void> {
  field().value = text;
  field().dispatchEvent(new Event("input"));
  await nextTick();
}

function press(key: string): void {
  field().dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
}

/** Mounts the bar with a library and a history behind it, the way the workspace mounts it. */
async function mount(library: readonly Snippet[] = [], history: readonly CommandHistoryEntry[] = []): Promise<void> {
  const commands = useCommandsStore();
  commands.history = [...history];
  commands.groups = library.length === 0 ? [] : [{ id: "default", name: "", order: 0, commands: [...library] }];

  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(CommandInputBar) }).use(i18n);
  app.mount(host);
  await nextTick();
}

beforeEach(() => {
  setActivePinia(createPinia());
  document.body.innerHTML = "";
  forgetAccepted();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the command bar offering what it knows", () => {
  it("draws the list while a word is being typed, and says it is open", async () => {
    await mount([], ran("uptime"));
    await type("upt");

    expect(rows()).toHaveLength(1);
    expect(rows()[0]?.textContent).toContain("uptime");
    expect(field().getAttribute("aria-expanded")).toBe("true");
    // Nothing is chosen yet: the first row is the one Tab would take, and the highlight appears only
    // once somebody points at a row, so a highlight always means "this one".
    expect(selected()).toBeNull();
  });

  it("offers nothing for an argument of a command it knows no words for", async () => {
    await mount([], ran("uptime"));
    await type("uptime -p");

    expect(rows()).toHaveLength(0);
    expect(field().getAttribute("aria-expanded")).toBe("false");
  });

  it("offers what a command it does know is followed by, once the first argument is being typed", async () => {
    await mount();
    await type("ls -");

    // A row reads "label" then its one line, so a label is checked as the start of the row.
    const labels = rows().map((row) => row.textContent?.trim() ?? "");
    expect(labels.some((label) => label.startsWith("-l"))).toBe(true);
  });

  it("moves through the rows with the arrows and wraps at both ends", async () => {
    await mount([snippet("解压", "unzip -o release.zip")], ran("uptime"));
    await type("u");

    const drawn = rows();
    expect(drawn.length).toBeGreaterThan(2);
    expect(selected(), "the list is up with nothing pointed at").toBeNull();

    press("ArrowDown");
    await nextTick();
    expect(selected(), "the first arrow key lands on the first row").toBe(drawn[0]?.textContent?.trim());

    press("ArrowDown");
    await nextTick();
    expect(selected()).toBe(drawn[1]?.textContent?.trim());

    press("ArrowUp");
    press("ArrowUp");
    await nextTick();
    expect(selected(), "up past the first row wraps to the last").toBe(drawn.at(-1)?.textContent?.trim());
  });

  it("takes the row Tab is on, writes it into the line and leaves the caret after it", async () => {
    await mount([], ran("uptime"));
    await type("upt");
    press("Tab");
    await nextTick();

    // One word out of the history is a word, not a line: it is left ready for the next word, the same
    // way the catalogue's `uptime` is.
    expect(field().value).toBe("uptime ");
    expect(field().selectionStart).toBe(7);
    expect(rows()).toHaveLength(0);
  });

  it("takes a line of more than one word as it stands, with nothing after it", async () => {
    await mount([], ran("uptime -p"));
    await type("uptim");
    press("Tab");
    await nextTick();

    expect(field().value).toBe("uptime -p");
    expect(field().selectionStart).toBe(9);
  });

  it("takes the row a click lands on", async () => {
    await mount([], ran("uptime"));
    await type("upt");
    rows()[0]?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await nextTick();

    expect(field().value).toBe("uptime ");
  });

  it("runs the line somebody typed when Enter is pressed, not the row the list is on", async () => {
    const commands = useCommandsStore();
    const send = vi.spyOn(commands, "sendLiteral").mockResolvedValue();
    await mount([], ran("uptime", "uptimed"));

    await type("upt");
    expect(rows().length, "there is a row that is not what was typed").toBeGreaterThan(1);

    press("Enter");
    await nextTick();

    expect(send).toHaveBeenCalledWith("upt");
    expect(field().value).toBe("");
    expect(rows()).toHaveLength(0);
  });

  it("stays shut for a line Escape refused, and answers the next one", async () => {
    await mount([], ran("uptime"));
    await type("upt");
    press("Escape");
    await nextTick();
    expect(rows()).toHaveLength(0);

    await type("upt");
    expect(rows(), "the same line again is the same question").toHaveLength(0);

    await type("upti");
    expect(rows()).toHaveLength(1);
  });

  it("leaves the candidate list alone while an input method is composing", async () => {
    await mount([], ran("uptime"));
    await type("upt");
    expect(rows()).toHaveLength(1);

    field().dispatchEvent(new Event("compositionstart"));
    await nextTick();
    expect(rows(), "the list gets out of the way while the IME is writing").toHaveLength(0);

    press("Tab");
    await nextTick();
    expect(field().value, "Tab belongs to the input method while it is composing").toBe("upt");

    field().dispatchEvent(new Event("compositionend"));
    await nextTick();
    expect(rows(), "and the list comes back when it is done").toHaveLength(1);
  });

  it("closes when the field loses the keyboard", async () => {
    await mount([], ran("uptime"));
    await type("upt");

    field().dispatchEvent(new Event("blur"));
    await nextTick();
    expect(rows()).toHaveLength(0);
  });

  it("opens what comes after a command once the command has been taken", async () => {
    await mount();
    await type("systemctl");
    press("Tab");
    // Two ticks: the caret is put back and the next word is opened for in the one that follows the
    // value being written, and the rows are drawn after that.
    await nextTick();
    await nextTick();

    expect(field().value).toBe("systemctl ");
    // The step after taking a word is the moment somebody has said what they are doing, so the next
    // word is already on offer instead of waiting to be asked for.
    const labels = rows().map((row) => row.textContent?.trim() ?? "");
    expect(labels.some((label) => label.startsWith("restart"))).toBe(true);
  });

  it("says nothing more after a whole line, which has no next word", async () => {
    await mount([], ran("systemctl restart nginx"));
    await type("system");
    press("Tab");
    await nextTick();

    expect(field().value).toBe("systemctl restart nginx");
    expect(rows()).toHaveLength(0);
  });

  it("puts the row the arrow keys were walked to into the line, and runs it on the next Enter", async () => {
    const commands = useCommandsStore();
    const send = vi.spyOn(commands, "sendLiteral").mockResolvedValue();
    await mount([], ran("cd ..", "cd /tmp"));

    // The word being typed is the command itself here, where the lines somebody has run are offered
    // beside it — after the exact match on the command, which is what the first row is.
    await type("cd");
    expect(rows().length, "the lines `cd` has been").toBeGreaterThan(1);

    press("ArrowDown");
    press("ArrowDown");
    await nextTick();
    expect(selected()).toContain("cd ..");

    press("Enter");
    await nextTick();

    expect(field().value, "the row goes into the line, which is usually not finished yet").toBe("cd ..");
    expect(send, "taking a row runs nothing").not.toHaveBeenCalled();

    press("Enter");
    await nextTick();

    expect(send, "and the next Enter runs what the line now says").toHaveBeenCalledWith("cd ..");
    expect(field().value).toBe("");
  });

  it("opens with nothing chosen, so a highlight always means somebody pointed at it", async () => {
    await mount([], ran("uptime"));
    await type("upt");

    expect(rows()).toHaveLength(1);
    expect(selected(), "the list is up but no row is chosen").toBeNull();
    expect(field().getAttribute("aria-activedescendant")).toBeNull();

    press("ArrowDown");
    await nextTick();
    expect(selected()).toContain("uptime");
    expect(field().getAttribute("aria-activedescendant")).not.toBeNull();
  });

  it("highlights the row under the pointer without letting Enter take it over", async () => {
    const commands = useCommandsStore();
    const send = vi.spyOn(commands, "sendLiteral").mockResolvedValue();
    await mount([], ran("uptime"));

    await type("upt");
    rows()[0]?.dispatchEvent(new MouseEvent("mousemove", { bubbles: true }));
    await nextTick();
    expect(selected(), "the row under the pointer is the one named").toContain("uptime");

    press("Enter");
    await nextTick();

    // A pointer rests over the list while somebody is typing at the line, so it is not a choice: the
    // arrow keys are. `touch a b` becoming `touch a a` is what this cost once.
    expect(send, "the line that was typed is what runs").toHaveBeenCalledWith("upt");
  });

  it("stays away when somebody has turned the list off", async () => {
    await mount([], ran("uptime"));
    usePreferencesStore().commandCompletion = false;

    await type("upt");
    expect(rows()).toHaveLength(0);

    press("Tab");
    await nextTick();
    expect(field().value, "Tab is the browser's again while the list is off").toBe("upt");
  });
});

describe("what the bar remembers", () => {
  it("notes a row somebody took, so the next list can put it higher", async () => {
    await mount([], ran("uptime"));
    await type("upt");
    press("Tab");
    await nextTick();

    expect(acceptanceBoost("uptime")).toBeGreaterThan(0);
  });
});
