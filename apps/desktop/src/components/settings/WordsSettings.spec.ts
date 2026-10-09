/**
 * The words page: what somebody added to what follows a command.
 *
 * The store's own rules are tested in its own spec, and the joining of their words with the ones
 * this build ships in `lib/commands/words.spec.ts`. This file is about the surface — the table, the
 * dialog that writes a word, and the row that edits or deletes one.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  completionWords: vi.fn(),
  completionWordSave: vi.fn(),
  completionWordDelete: vi.fn(),
}));

import WordsSettings from "@/components/settings/WordsSettings.vue";
import { i18n } from "@/i18n";
import { completionWordDelete, completionWordSave, completionWords } from "@/lib/backend/tauri";
import type { CompletionWord } from "@/types/snippet";

const listMock = vi.mocked(completionWords);
const saveMock = vi.mocked(completionWordSave);
const deleteMock = vi.mocked(completionWordDelete);

let app: App | undefined;
let host: HTMLDivElement;

function word(command: string, text: string, note = ""): CompletionWord {
  return { id: `word-${command}-${text}`, command, word: text, note };
}

async function mount(stored: readonly CompletionWord[] = [], then: readonly CompletionWord[] = stored): Promise<void> {
  listMock.mockReset();
  listMock.mockResolvedValueOnce([...stored]);
  listMock.mockResolvedValue([...then]);

  host = document.createElement("div");
  document.body.append(host);
  app = createApp({ render: () => h(WordsSettings) }).use(i18n);
  app.mount(host);
  await vi.waitFor(() => {
    expect(host.querySelector("table"), "the page has read what is stored").not.toBeNull();
  });
  await nextTick();
}

/** The dialog is drawn in a portal, so it lies beside the page rather than inside it. */
function dialog(): HTMLElement | null {
  return document.body.querySelector<HTMLElement>('[role="dialog"]');
}

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

async function type(id: string, value: string): Promise<void> {
  const element = document.body.querySelector<HTMLInputElement>(`#${id}`);
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

describe("the words page", () => {
  it("lists a word with the command it follows and what it means", async () => {
    await mount([word("kubectl", "apply", "收起那份清单")]);

    const row = host.querySelector("[data-word-row]");
    expect(row?.textContent).toContain("kubectl");
    expect(row?.textContent).toContain("apply");
    expect(row?.textContent).toContain("收起那份清单");
  });

  it("lists what this build ships beside what somebody added", async () => {
    // The page is also where somebody looks to see what is already offered, so the shipped words are
    // in it — named as shipped, and with nothing to press on their rows.
    await mount([word("acme", "drain", "my own")]);

    const rows = [...host.querySelectorAll("[data-word-row]")];
    expect(rows.length, "the shipped table is longer than one row").toBeGreaterThan(10);
    expect(rows[0]?.textContent, "their own row is first").toContain("acme");
    expect(rows[0]?.textContent).toContain("Yours");
    expect(rows[0]?.querySelector("button"), "it has actions").not.toBeNull();

    const shipped = rows.find((row) => row.textContent?.includes("Yours") === false);
    expect(shipped?.textContent).toContain("Built in");
    expect(shipped?.querySelector("button"), "a shipped word has nothing to press").toBeNull();
  });

  it("leaves a shipped word out when somebody has their own row for it", async () => {
    await mount([word("git", "status", "mine")]);

    const rows = [...host.querySelectorAll("[data-word-row]")];
    // Two words in the shipped table say `status` — `git status` and `systemctl status` — so the row
    // is picked out by both halves of the pair it is about.
    const status = rows.filter((row) => row.textContent?.includes("git") && row.textContent?.includes("status"));
    expect(status, "one row for one word").toHaveLength(1);
    expect(status[0]?.textContent).toContain("Yours");
  });

  it("writes what the dialog is given", async () => {
    saveMock.mockResolvedValue(word("kubectl", "apply"));
    await mount();
    await press("Add a word");

    expect(dialog(), "adding is a dialog").not.toBeNull();
    await type("word-command", "kubectl");
    await type("word-word", "apply");
    await type("word-note", "收起那份清单");
    await press("Save", dialog() as ParentNode);

    expect(saveMock).toHaveBeenCalledWith({ id: "", command: "kubectl", word: "apply", note: "收起那份清单" });
    await vi.waitFor(() => {
      expect(dialog(), "the dialog goes once the word is written").toBeNull();
    });
  });

  it("changes the word a row already has, keeping its id", async () => {
    const stored = word("kubectl", "aply");
    saveMock.mockResolvedValue({ ...stored, word: "apply" });
    await mount([stored]);

    await press(/^Edit: /);
    await type("word-word", "apply");
    await press("Save", dialog() as ParentNode);

    expect(saveMock).toHaveBeenCalledWith({ id: stored.id, command: "kubectl", word: "apply", note: "" });
  });

  it("will not save a word that is two words", async () => {
    await mount();
    await press("Add a word");
    await type("word-command", "kubectl");
    await type("word-word", "apply now");

    const save = (): HTMLButtonElement | undefined =>
      [...(dialog()?.querySelectorAll("button") ?? [])].find((button) => button.textContent?.trim() === "Save");
    expect(save()?.disabled, "the command line is split on spaces before anything is looked up").toBe(true);
    expect(dialog()?.textContent).toContain("one word each");
  });

  it("deletes the word a row is about", async () => {
    const stored = word("kubectl", "apply");
    deleteMock.mockResolvedValue();
    await mount([stored], []);

    await press(/^Delete: /);
    await vi.waitFor(() => {
      expect(deleteMock).toHaveBeenCalledWith(stored.id);
    });
  });
});
