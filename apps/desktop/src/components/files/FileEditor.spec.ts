/**
 * The editor's one exit, and the question that stands in it.
 *
 * Three ways out used to close the editor outright — Cancel, Escape, a click on the overlay — and
 * each of them threw away whatever had been typed. What is asserted here is that all three now lead
 * to the same question, that the question has three answers rather than two, and that "keep
 * editing" really does keep the buffer: a decision that was made by pressing Escape must not be the
 * one that loses the work.
 */
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  filesReadText: vi.fn(async () => "one line"),
  filesWriteText: vi.fn(async () => {}),
}));

import FileEditor from "@/components/files/FileEditor.vue";
import { i18n } from "@/i18n";
import { filesReadText, filesWriteText } from "@/lib/backend/tauri";
import { useFilesStore } from "@/stores/files";

const readText = vi.mocked(filesReadText);
const writeText = vi.mocked(filesWriteText);

let app: App | undefined;
let host: HTMLDivElement;
let closed: boolean[];

async function render(): Promise<void> {
  app = createApp({
    data: () => ({ open: true }),
    render() {
      return h(FileEditor, {
        open: this.open,
        handle: "browser-1",
        path: "/etc/nginx/nginx.conf",
        "onUpdate:open": (value: boolean) => {
          closed.push(value);
          this.open = value;
        },
      });
    },
  }).use(i18n);

  host = document.createElement("div");
  document.body.append(host);
  app.mount(host);
  await settle();
}

/** The editor's own dialog: the first one on screen, since a question would be the second. */
function editor(): HTMLElement {
  const element = document.querySelector<HTMLElement>('[role="dialog"]');
  if (!element) {
    throw new Error("the editor was not drawn");
  }
  return element;
}

function question(): HTMLElement | null {
  const dialogs = [...document.querySelectorAll<HTMLElement>('[role="dialog"]')];
  return dialogs.length > 1 ? (dialogs[1] ?? null) : null;
}

function field(): HTMLTextAreaElement {
  const element = editor().querySelector<HTMLTextAreaElement>("textarea");
  if (!element) {
    throw new Error("the editor has no field");
  }
  return element;
}

function buttonNamed(scope: HTMLElement, text: string): HTMLButtonElement {
  const element = [...scope.querySelectorAll<HTMLButtonElement>("button")].find(
    (candidate) => candidate.textContent?.trim() === text,
  );
  if (!element) {
    throw new Error(`no button says ${text}`);
  }
  return element;
}

/** Typing, which is what makes the buffer dirty. */
async function type(value: string): Promise<void> {
  const textarea = field();
  textarea.value = value;
  textarea.dispatchEvent(new Event("input"));
  await settle();
}

async function settle(): Promise<void> {
  await nextTick();
  await nextTick();
  await new Promise((resolve) => setTimeout(resolve, 0));
}

beforeEach(() => {
  setActivePinia(createPinia());
  closed = [];
  readText.mockClear().mockResolvedValue("one line");
  writeText.mockClear().mockResolvedValue(undefined);
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
  for (const node of document.querySelectorAll('[role="dialog"]')) {
    node.remove();
  }
});

describe("opening and saving", () => {
  it("reads the file through the backend", async () => {
    await render();
    expect(readText).toHaveBeenCalledWith("browser-1", "/etc/nginx/nginx.conf");
    expect(field().value).toBe("one line");
  });

  it("writes what was typed and closes", async () => {
    await render();
    await type("two lines");

    buttonNamed(editor(), "Save").click();
    await settle();

    expect(writeText).toHaveBeenCalledWith("browser-1", "/etc/nginx/nginx.conf", "two lines");
    expect(closed).toEqual([false]);
  });
});

describe("closing with nothing to lose", () => {
  it("closes without a question when nothing was typed", async () => {
    await render();

    buttonNamed(editor(), "Cancel").click();
    await settle();

    expect(closed).toEqual([false]);
    expect(question()).toBeNull();
  });
});

describe("closing with unsaved changes", () => {
  it("asks, with all three answers, and stays open until one is given", async () => {
    await render();
    await type("two lines");

    buttonNamed(editor(), "Cancel").click();
    await settle();

    const asked = question();
    expect(asked, "the question is a dialog of its own").not.toBeNull();
    expect(asked?.textContent).toContain("Unsaved changes");
    expect(buttonNamed(asked as HTMLElement, "Keep editing")).toBeTruthy();
    expect(buttonNamed(asked as HTMLElement, "Discard changes")).toBeTruthy();
    expect(buttonNamed(asked as HTMLElement, "Save and close")).toBeTruthy();
    expect(closed, "nothing has been decided").toEqual([]);
  });

  it("keeps the work when the answer is to keep editing", async () => {
    await render();
    await type("two lines");

    buttonNamed(editor(), "Cancel").click();
    await settle();
    buttonNamed(question() as HTMLElement, "Keep editing").click();
    await settle();

    expect(closed).toEqual([]);
    expect(field().value).toBe("two lines");
  });

  it("loses it only when throwing it away is the answer", async () => {
    await render();
    await type("two lines");

    buttonNamed(editor(), "Cancel").click();
    await settle();
    buttonNamed(question() as HTMLElement, "Discard changes").click();
    await settle();

    expect(writeText).not.toHaveBeenCalled();
    expect(closed).toEqual([false]);
  });

  it("writes it out when saving is the answer", async () => {
    await render();
    await type("two lines");

    buttonNamed(editor(), "Cancel").click();
    await settle();
    buttonNamed(question() as HTMLElement, "Save and close").click();
    await settle();

    expect(writeText).toHaveBeenCalledWith("browser-1", "/etc/nginx/nginx.conf", "two lines");
    expect(closed).toEqual([false]);
  });

  it("keeps the editor, with the reason, when the write fails", async () => {
    writeText.mockRejectedValue(new Error("the disk is full"));
    await render();
    await type("two lines");

    buttonNamed(editor(), "Cancel").click();
    await settle();
    buttonNamed(question() as HTMLElement, "Save and close").click();
    await settle();

    expect(closed).toEqual([]);
    expect(question(), "the question stands aside for the failure").toBeNull();
    expect(editor().textContent).toContain("APP-INTERNAL");
  });

  it("takes Escape as the question too, rather than as a way out", async () => {
    await render();
    await type("two lines");

    editor().dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
    await settle();

    expect(closed).toEqual([]);
    expect(question()).not.toBeNull();
  });
});

/**
 * The shell has to know that a window would be closed over unsaved text, and a dialog is not
 * something it can look inside — so the editor says so itself, and unsays it the moment there is
 * nothing left to lose.
 */
describe("telling the shell what is at stake", () => {
  it("names itself while it holds something unsaved, and stops when it does not", async () => {
    await render();
    const files = useFilesStore();
    expect(files.unsavedEditors).toEqual([]);

    await type("two lines");
    expect(files.unsavedEditors).toEqual(["browser-1:/etc/nginx/nginx.conf"]);

    buttonNamed(editor(), "Save").click();
    await settle();
    expect(files.unsavedEditors).toEqual([]);
  });

  it("stops saying so when the editor goes away with the text still unsaved", async () => {
    await render();
    const files = useFilesStore();
    await type("two lines");
    expect(files.unsavedEditors).toHaveLength(1);

    app?.unmount();
    app = undefined;

    expect(files.unsavedEditors).toEqual([]);
  });
});
