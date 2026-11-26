import { EditorSelection } from "@codemirror/state";
import { createPinia, setActivePinia } from "pinia";
import { type App, createApp, h, nextTick, ref } from "vue";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import TextEditor from "@/components/ui/TextEditor.vue";
import { i18n } from "@/i18n";
import { pressEditorKey, textEditorView } from "@/lib/editor/testing";
import { useAppStore } from "@/stores/app";
import { usePreferencesStore } from "@/stores/preferences";

let app: App | undefined;
let host: HTMLDivElement;
const text = ref("");
const readOnly = ref(false);
const busy = ref(false);
const saved = vi.fn();
const submitted = vi.fn();
const cancelled = vi.fn();

async function mount(value: string, yaml = false): Promise<void> {
  text.value = value;
  app = createApp({
    render: () =>
      h(TextEditor, {
        modelValue: text.value,
        "onUpdate:modelValue": (value: string) => {
          text.value = value;
        },
        label: "Test document",
        readonly: readOnly.value,
        busy: busy.value,
        yaml,
        onSave: saved,
        onSubmit: submitted,
        onCancel: cancelled,
      }),
  });
  app.use(i18n).mount(host);
  await nextTick();
}

beforeEach(() => {
  setActivePinia(createPinia());
  host = document.createElement("div");
  document.body.append(host);
  readOnly.value = false;
  busy.value = false;
  saved.mockClear();
  submitted.mockClear();
  cancelled.mockClear();
});

afterEach(() => {
  app?.unmount();
  app = undefined;
  host.remove();
});

describe("the editor's indentation keys", () => {
  it("keeps focus and inserts one step at the actual cursor", async () => {
    await mount("ab");
    const view = textEditorView(host);
    view.dispatch({ selection: { anchor: 1 } });
    const event = pressEditorKey(host, "Tab");
    await nextTick();
    expect(event.defaultPrevented).toBe(true);
    expect(text.value).toBe("a    b");
    expect(view.state.selection.main.head).toBe(5);
    expect(document.activeElement).toBe(view.contentDOM);
  });

  it("indents and outdents a block without including the following line", async () => {
    await mount("one\ntwo\nthree");
    const view = textEditorView(host);
    view.dispatch({ selection: EditorSelection.range(0, 8) });
    pressEditorKey(host, "Tab");
    await nextTick();
    expect(text.value).toBe("    one\n    two\nthree");
    pressEditorKey(host, "Tab", { shiftKey: true });
    await nextTick();
    expect(text.value).toBe("one\ntwo\nthree");
    expect(view.state.selection.main.empty).toBe(false);
  });

  it("removes only existing indentation from a partly indented line", async () => {
    await mount("  one");
    textEditorView(host).dispatch({ selection: { anchor: 5 } });
    pressEditorKey(host, "Tab", { shiftKey: true });
    await nextTick();
    expect(text.value).toBe("one");
  });

  it("keeps the draft, selection and undo history when indentation preferences change", async () => {
    await mount("ab");
    const view = textEditorView(host);
    view.dispatch({ selection: { anchor: 1 } });
    pressEditorKey(host, "Tab");
    await nextTick();
    const selection = view.state.selection;
    const preferences = usePreferencesStore();
    preferences.setEditorIndentWidth(2);
    preferences.setEditorIndentStyle("tabs");
    await nextTick();
    expect(textEditorView(host)).toBe(view);
    expect(view.state.selection.eq(selection)).toBe(true);
    expect(text.value).toBe("a    b");
    pressEditorKey(host, "z", { ctrlKey: true });
    await nextTick();
    expect(text.value).toBe("ab");
    pressEditorKey(host, "z", { ctrlKey: true, shiftKey: true });
    await nextTick();
    expect(text.value).toBe("a    b");
    pressEditorKey(host, "Tab");
    await nextTick();
    expect(text.value).toBe("a    \tb");
    expect(view.state.tabSize).toBe(2);
  });

  it("uses spaces in YAML even when ordinary text uses Tabs", async () => {
    const preferences = usePreferencesStore();
    preferences.setEditorIndentStyle("tabs");
    preferences.setEditorIndentWidth(2);
    await mount("key: value", true);
    pressEditorKey(host, "Tab");
    await nextTick();
    expect(text.value).toBe("  key: value");
    expect(host.textContent).toContain("YAML uses spaces");
  });

  it("leaves Tab available for focus navigation after the mode is switched", async () => {
    await mount("keep");
    pressEditorKey(host, "m", { ctrlKey: true, shiftKey: true });
    await nextTick();
    expect(host.textContent).toContain("Tab: move focus");
    expect(pressEditorKey(host, "Tab").defaultPrevented).toBe(false);
    expect(pressEditorKey(host, "Tab", { shiftKey: true }).defaultPrevented).toBe(false);
    expect(text.value).toBe("keep");
    pressEditorKey(host, "m", { ctrlKey: true, shiftKey: true });
    expect(pressEditorKey(host, "Tab").defaultPrevented).toBe(true);
    await nextTick();
    expect(text.value).toBe("    keep");
  });

  it("does not capture Ctrl+Tab as an indentation key", async () => {
    await mount("keep");
    expect(pressEditorKey(host, "Tab", { ctrlKey: true }).defaultPrevented).toBe(false);
    expect(text.value).toBe("keep");
  });
});

describe("newlines and input methods", () => {
  it("does not inherit indentation until the setting is enabled", async () => {
    await mount("  key:");
    const view = textEditorView(host);
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    pressEditorKey(host, "Enter");
    await nextTick();
    expect(text.value).toBe("  key:\n");
  });

  it("inherits leading whitespace without guessing a language's block structure", async () => {
    usePreferencesStore().setEditorAutoIndent(true);
    await mount("  key:");
    const view = textEditorView(host);
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    pressEditorKey(host, "Enter");
    await nextTick();
    expect(text.value).toBe("  key:\n  ");
  });

  it("writes inherited YAML indentation as spaces even if the preceding line contained a pasted Tab", async () => {
    usePreferencesStore().setEditorAutoIndent(true);
    await mount("\tkey:", true);
    const view = textEditorView(host);
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    pressEditorKey(host, "Enter");
    await nextTick();
    expect(text.value).toBe("\tkey:\n    ");
  });

  it("keeps CRLF when the edited buffer is emitted for saving", async () => {
    await mount("a\r\nb\r\n");
    textEditorView(host).dispatch({ selection: { anchor: 1 } });
    pressEditorKey(host, "Tab");
    await nextTick();
    expect(text.value).toBe("a    \r\nb\r\n");
  });

  it("accepts LF text pasted into a CRLF document without embedding newlines inside logical lines", async () => {
    await mount("a\r\nb");
    const view = textEditorView(host);
    view.dispatch({ selection: { anchor: view.state.doc.length } });
    view.dispatch(view.state.replaceSelection("\nc\nd"));
    await nextTick();
    expect(view.state.doc.lines).toBe(4);
    expect(text.value).toBe("a\r\nb\r\nc\r\nd");
  });

  it("leaves composition keys to the input method instead of indenting or submitting", async () => {
    await mount("keep");
    expect(pressEditorKey(host, "Tab", { isComposing: true }).defaultPrevented).toBe(false);
    expect(pressEditorKey(host, "Enter", { isComposing: true }).defaultPrevented).toBe(false);
    expect(pressEditorKey(host, "Enter", { ctrlKey: true, isComposing: true }).defaultPrevented).toBe(false);
    await nextTick();
    expect(text.value).toBe("keep");
    expect(saved).not.toHaveBeenCalled();
    expect(submitted).not.toHaveBeenCalled();
  });
});

describe("the editor's action shortcuts", () => {
  it("dispatches save, submit and cancel only inside the editor", async () => {
    await mount("keep");
    const outer = vi.fn();
    window.addEventListener("keydown", outer);
    try {
      pressEditorKey(host, "s", { ctrlKey: true });
      pressEditorKey(host, "Enter", { ctrlKey: true });
      pressEditorKey(host, "Escape");
      expect(saved).toHaveBeenCalledTimes(1);
      expect(submitted).toHaveBeenCalledTimes(1);
      expect(cancelled).toHaveBeenCalledTimes(1);
      expect(text.value).toBe("keep");
      expect(outer).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("keydown", outer);
    }
  });

  it("uses the current shortcut override without remounting the editor", async () => {
    await mount("keep");
    const view = textEditorView(host);
    usePreferencesStore().setKeybinding("editor.save", "Mod+Shift+S");
    await nextTick();
    pressEditorKey(host, "s", { ctrlKey: true });
    expect(saved).not.toHaveBeenCalled();
    pressEditorKey(host, "s", { ctrlKey: true, shiftKey: true });
    expect(saved).toHaveBeenCalledTimes(1);
    expect(textEditorView(host)).toBe(view);
  });

  it("uses Command on macOS and does not treat Control as that key", async () => {
    vi.spyOn(useAppStore(), "platform", "get").mockReturnValue("macos aarch64");
    await mount("keep");
    pressEditorKey(host, "s", { ctrlKey: true });
    expect(saved).not.toHaveBeenCalled();
    pressEditorKey(host, "s", { metaKey: true });
    expect(saved).toHaveBeenCalledTimes(1);
  });

  it("keeps read-only text intact and consumes save without sending anything", async () => {
    readOnly.value = true;
    await mount("keep");
    expect(pressEditorKey(host, "Tab").defaultPrevented).toBe(false);
    pressEditorKey(host, "Enter");
    pressEditorKey(host, "s", { ctrlKey: true });
    expect(text.value).toBe("keep");
    expect(saved).not.toHaveBeenCalled();
    expect(textEditorView(host).contentDOM.getAttribute("aria-readonly")).toBe("true");
  });

  it("does not cancel, submit or save again while the owner is busy", async () => {
    busy.value = true;
    await mount("keep");
    pressEditorKey(host, "s", { ctrlKey: true });
    pressEditorKey(host, "Enter", { ctrlKey: true });
    pressEditorKey(host, "Escape");
    expect(saved).not.toHaveBeenCalled();
    expect(submitted).not.toHaveBeenCalled();
    expect(cancelled).not.toHaveBeenCalled();
    busy.value = false;
    await nextTick();
    pressEditorKey(host, "s", { ctrlKey: true, repeat: true });
    expect(saved).not.toHaveBeenCalled();
    pressEditorKey(host, "s", { ctrlKey: true });
    expect(saved).toHaveBeenCalledTimes(1);
  });
});
