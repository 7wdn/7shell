/**
 * The command library on the settings page: the same list the panel under a terminal edits.
 *
 * The store's own rules are tested where they live. This file is about the surface — that the rows
 * show what is saved with the trigger and the line, that add and edit both go through the dialog the
 * store's draft is written in, and that the switch on a row writes the command it belongs to.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  snippetsOpen: vi.fn(),
  snippetSave: vi.fn(),
  snippetDelete: vi.fn(),
  snippetGroupSave: vi.fn(),
  snippetGroupDelete: vi.fn(),
  commandHistory: vi.fn(),
  commandHistoryClear: vi.fn(),
  commandClassify: vi.fn(),
  commandSend: vi.fn(),
  sessionOpen: vi.fn(),
  terminalWrite: vi.fn(),
}));

import CommandsLibrary from "@/components/settings/CommandsLibrary.vue";
import { BUILTIN_COMMANDS } from "@/lib/commands/builtin";
import { i18n } from "@/i18n";
import { commandHistory, snippetSave, snippetsOpen } from "@/lib/backend/tauri";
import { useCommandsStore } from "@/stores/commands";
import type { Snippet, SnippetGroup } from "@/types/snippet";

const listMock = vi.mocked(snippetsOpen);
const historyMock = vi.mocked(commandHistory);
const saveMock = vi.mocked(snippetSave);

let app: App | undefined;
let host: HTMLDivElement;

function snippet(name: string, prefix: string, command: string, enabled = true): Snippet {
  return { id: name, groupId: "default", name, prefix, command, appendCr: true, enabled, order: 0, slotHistory: [] };
}

function library(commands: readonly Snippet[]): SnippetGroup[] {
  return [{ id: "default", name: "", order: 0, commands: [...commands] }];
}

/**
 * One of the lines this build ships, so the library under test looks like one that already has them.
 *
 * Without it every read looks like a fresh install and the seeding writes twenty commands behind the
 * test — which is a fact about the seeding, not about the switch a test is trying to press.
 */
function shipped(): Snippet {
  const line = BUILTIN_COMMANDS[0] ?? { name: "", prefix: "", command: "" };
  return snippet(line.name, line.prefix, line.command);
}

/**
 * Mounts the page with a library behind it.
 *
 * `then` is what every later read answers with, which is how a write is seen to come back: a fresh
 * install passes an empty library and what the seeding is expected to have written behind it.
 */
async function mount(commands: readonly Snippet[] = [], then: readonly Snippet[] = commands): Promise<void> {
  listMock.mockReset();
  listMock.mockResolvedValueOnce(library(commands));
  listMock.mockResolvedValue(library(then));
  historyMock.mockResolvedValue([]);

  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(CommandsLibrary) }).use(i18n);
  app.mount(host);
  await nextTick();
}

/** The rows are drawn once the page has read the library and opened a group. */
async function rows(count = 1): Promise<void> {
  await vi.waitFor(() => {
    expect(host.querySelectorAll("[data-command-row]").length).toBe(count);
  });
}

/** The dialog is drawn in a portal, so it lies beside the page rather than inside it. */
function dialog(): HTMLElement | null {
  return document.body.querySelector<HTMLElement>('[role="dialog"]');
}

function field(id: string): HTMLInputElement | HTMLTextAreaElement | null {
  return document.body.querySelector<HTMLInputElement | HTMLTextAreaElement>(`#${id}`);
}

/** Presses a button by what it reads, or by the name it gives a screen reader when it draws nothing. */
async function press(label: string | RegExp, where: ParentNode = host): Promise<void> {
  const button = [...where.querySelectorAll("button")].find((candidate) => {
    const reading = candidate.textContent?.trim() ?? "";
    const named = candidate.getAttribute("aria-label") ?? "";
    return typeof label === "string" ? reading === label || named === label : label.test(reading) || label.test(named);
  });
  if (!button) {
    throw new Error(`no button reading ${String(label)}`);
  }
  button.click();
  await nextTick();
}

/** Types into a field the way a person does, so the store's draft is what changes. */
async function type(id: string, value: string): Promise<void> {
  const element = field(id);
  if (!element) {
    throw new Error(`no field ${id}`);
  }
  element.value = value;
  element.dispatchEvent(new Event("input"));
  await nextTick();
}

beforeEach(() => {
  setActivePinia(createPinia());
  document.body.innerHTML = "";
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the command library page", () => {
  it("lists what is saved with the trigger and the line", async () => {
    await mount([snippet("重启 nginx", "res", "systemctl restart #1")]);
    await rows();

    expect(host.textContent).toContain("重启 nginx");
    expect(host.textContent).toContain("res");
    expect(host.textContent).toContain("systemctl restart #1");
  });

  it("says so when a command has no trigger, rather than showing an empty cell", async () => {
    await mount([snippet("开机时长", "", "uptime")]);
    await rows();

    expect(host.textContent).toContain("No trigger");
  });

  it("shows the commands this build ships when there is nothing saved yet", async () => {
    await mount([], [snippet("df -h", "dfh", "df -h"), snippet("uptime", "up", "uptime")]);
    await rows(2);

    expect(host.textContent).toContain("df -h");
    expect(saveMock, "they are written into the library rather than kept apart from it").toHaveBeenCalled();
  });

  it("shows them beside what somebody saved themselves", async () => {
    // Somebody who has been using the application has lines of their own, and the shipped list has to
    // reach them too: a library is not "fresh" just because it is not empty.
    const mine = snippet("重启 nginx", "res", "systemctl restart #1");
    await mount([mine], [mine, snippet("df -h", "dfh", "df -h")]);
    await rows(2);

    expect(host.textContent).toContain("重启 nginx");
    expect(host.textContent).toContain("df -h");
  });

  it("names the columns of its table", async () => {
    await mount([snippet("重启 nginx", "res", "systemctl restart #1")]);
    await rows();

    const headings = [...host.querySelectorAll("thead th")].map((cell) => cell.textContent?.trim());
    expect(headings).toEqual(["Name", "Trigger", "Status", "The line", "Actions"]);
  });

  it("says a command is switched off on its own row", async () => {
    await mount([snippet("重启 nginx", "res", "systemctl restart #1", false)]);
    await rows();

    expect(host.textContent).toContain("Switched off");
  });

  it("opens the dialog on Add, and the store's draft is what it edits", async () => {
    await mount([snippet("重启 nginx", "res", "systemctl restart #1")]);
    await rows();

    await press("Add a command");

    expect(dialog(), "adding is a dialog, not a form in the page").not.toBeNull();
    expect(useCommandsStore().draft).toMatchObject({ id: "", name: "", prefix: "", command: "" });
    expect(field("command-name")).not.toBeNull();
  });

  it("loads a command into the dialog when its row is edited", async () => {
    await mount([snippet("重启 nginx", "res", "systemctl restart #1")]);
    await rows();

    await press(/^Edit: /);

    expect(dialog()).not.toBeNull();
    expect(useCommandsStore().draft).toMatchObject({
      id: "重启 nginx",
      name: "重启 nginx",
      prefix: "res",
      command: "systemctl restart #1",
    });
    expect(field("command-name")?.value).toBe("重启 nginx");
  });

  it("will not save a command without a name and a line", async () => {
    await mount();
    await nextTick();
    await press("Add a command");

    const save = (): HTMLButtonElement | undefined =>
      [...(dialog()?.querySelectorAll("button") ?? [])].find((button) => button.textContent?.trim() === "Save");
    expect(save()?.disabled, "nothing is written yet").toBe(true);

    await type("command-name", "重启 nginx");
    expect(save()?.disabled, "a name with no line is not a command").toBe(true);

    await type("command-line", "systemctl restart #1");
    expect(save()?.disabled).toBe(false);

    // A trigger is matched against one word, so a space in it is a trigger that could never answer.
    await type("command-trigger", "res tart");
    expect(save()?.disabled, "a trigger with a space in it is not one").toBe(true);
    expect(dialog()?.textContent).toContain("cannot contain a space");
  });

  it("writes a command straight away when its switch is turned", async () => {
    const stored = snippet("重启 nginx", "res", "systemctl restart #1");
    saveMock.mockResolvedValue({ ...stored, enabled: false });
    await mount([shipped(), stored]);
    await rows(2);

    // The switch is the library's own: turning it is a change to the command, not a form to submit.
    // Found by the name it gives a screen reader, because the page carries another switch of its own.
    const control = host.querySelector<HTMLElement>('[role="switch"][aria-label="Enabled: 重启 nginx"]');
    control?.click();
    await nextTick();

    expect(saveMock).toHaveBeenCalledWith({ ...stored, enabled: false });
  });

  it("carries the switch that governs the whole feature", async () => {
    await mount([shipped()]);
    await rows();

    // It sat under the terminal's settings until it moved here: it switches the menu beside the
    // command line, and this page is the one about that menu.
    const control = host.querySelector<HTMLElement>('[role="switch"][aria-label="Command completion"]');

    expect(control, "the completion switch is on the library's page").not.toBeNull();
  });
});
