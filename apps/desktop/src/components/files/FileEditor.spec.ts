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
import { type App, createApp, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/lib/runtime", () => ({ isDesktopRuntime: vi.fn(() => true) }));
vi.mock("@/lib/backend/tauri", () => ({
  filesReadText: vi.fn(async () => "one line"),
  filesWriteText: vi.fn(async () => {}),
}));

import FileEditor from "@/components/files/FileEditor.vue";
import { i18n } from "@/i18n";
import { filesReadText, filesWriteText } from "@/lib/backend/tauri";
import { pressEditorKey, replaceEditorText, textEditorView } from "@/lib/editor/testing";
import { useFilesStore } from "@/stores/files";

const readText = vi.mocked(filesReadText);
const writeText = vi.mocked(filesWriteText);

let app: App | undefined;
let host: HTMLDivElement;
let closed: boolean[];
const editedPath = ref("/etc/nginx/nginx.conf");

async function render(): Promise<void> {
  app = createApp({
    data: () => ({ open: true }),
    render() {
      return h(FileEditor, {
        open: this.open,
        handle: "browser-1",
        path: editedPath.value,
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

function field() {
  return textEditorView(editor());
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
  replaceEditorText(editor(), value);
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
  editedPath.value = "/etc/nginx/nginx.conf";
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
    expect(field().state.sliceDoc()).toBe("one line");
  });

  it("writes what was typed and closes", async () => {
    await render();
    await type("two lines");

    buttonNamed(editor(), "Save").click();
    await settle();

    expect(writeText).toHaveBeenCalledWith("browser-1", "/etc/nginx/nginx.conf", "two lines");
    expect(closed).toEqual([false]);
  });

  it("saves with Ctrl+S and keeps the editor and its buffer open", async () => {
    await render();
    await type("saved without closing");
    pressEditorKey(editor(), "s", { ctrlKey: true });
    await settle();
    expect(writeText).toHaveBeenCalledWith("browser-1", "/etc/nginx/nginx.conf", "saved without closing");
    expect(closed).toEqual([]);
    expect(field().state.sliceDoc()).toBe("saved without closing");
    expect(useFilesStore().unsavedEditors).toEqual([]);
  });

  it("saves and closes with Ctrl+Enter", async () => {
    await render();
    await type("saved and closed");
    pressEditorKey(editor(), "Enter", { ctrlKey: true });
    await settle();
    expect(writeText).toHaveBeenCalledWith("browser-1", "/etc/nginx/nginx.conf", "saved and closed");
    expect(closed).toEqual([false]);
  });

  it("does not write again or close while the first save is pending", async () => {
    let finish: (() => void) | undefined;
    writeText.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    await render();
    await type("one submission");
    pressEditorKey(editor(), "s", { ctrlKey: true });
    await settle();
    expect(field().state.readOnly).toBe(true);
    pressEditorKey(editor(), "s", { ctrlKey: true });
    pressEditorKey(editor(), "Enter", { ctrlKey: true });
    pressEditorKey(editor(), "Escape");
    await settle();
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(closed).toEqual([]);
    expect(question()).toBeNull();
    finish?.();
    await settle();
    expect(field().state.readOnly).toBe(false);
    expect(useFilesStore().unsavedEditors).toEqual([]);
  });

  it("keeps a keyboard save failure dirty and asks before Escape discards it", async () => {
    writeText.mockRejectedValueOnce(new Error("the disk is full"));
    await render();
    await type("unsaved text");
    pressEditorKey(editor(), "s", { ctrlKey: true });
    await settle();
    expect(field().state.sliceDoc()).toBe("unsaved text");
    expect(useFilesStore().unsavedEditors).toHaveLength(1);
    pressEditorKey(editor(), "Escape");
    await settle();
    expect(question()).not.toBeNull();
    expect(closed).toEqual([]);
  });

  it("does not let an old read replace the file opened after it", async () => {
    let finish: ((text: string) => void) | undefined;
    readText.mockImplementationOnce(
      () =>
        new Promise<string>((resolve) => {
          finish = resolve;
        }),
    );
    await render();
    readText.mockResolvedValueOnce("new file");
    editedPath.value = "/etc/nginx/new.conf";
    await settle();
    expect(field().state.sliceDoc()).toBe("new file");
    finish?.("old file");
    await settle();
    expect(field().state.sliceDoc()).toBe("new file");
  });

  it("does not close a different file when an earlier save and close finishes", async () => {
    let finish: (() => void) | undefined;
    writeText.mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          finish = resolve;
        }),
    );
    await render();
    await type("old draft");
    pressEditorKey(editor(), "Enter", { ctrlKey: true });
    await settle();
    readText.mockResolvedValueOnce("new file");
    editedPath.value = "/etc/nginx/new.conf";
    await settle();
    expect(useFilesStore().unsavedEditors).toEqual([]);
    finish?.();
    await settle();
    expect(closed).toEqual([]);
    expect(field().state.sliceDoc()).toBe("new file");
    expect(writeText).toHaveBeenCalledWith("browser-1", "/etc/nginx/nginx.conf", "old draft");
  });
});

describe("a file that could not be read", () => {
  /**
   * The buffer after a failed read is empty, and the save button used to be live. One click would
   * have written that empty buffer over the remote file — the editor destroying the very file it
   * had just said it could not show.
   */
  it("offers no way to write the empty buffer back over the file", async () => {
    readText.mockRejectedValueOnce(new Error("the file is not text"));
    await render();

    expect(editor().textContent).toContain("APP-INTERNAL");
    expect(field().state.readOnly, "there is nothing to type into").toBe(true);
    expect(buttonNamed(editor(), "Save").disabled, "an unread file must not be savable").toBe(true);

    pressEditorKey(editor(), "s", { ctrlKey: true });
    pressEditorKey(editor(), "Enter", { ctrlKey: true });

    buttonNamed(editor(), "Save").click();
    await settle();
    expect(writeText).not.toHaveBeenCalled();
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
    expect(field().state.sliceDoc()).toBe("two lines");
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
