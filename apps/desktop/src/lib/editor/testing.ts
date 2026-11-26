import { Text } from "@codemirror/state";
import { EditorView } from "@codemirror/view";

/** Finds the actual editor so component tests exercise its transactions and DOM key handlers. */
export function textEditorView(scope: ParentNode): EditorView {
  const dom = scope.querySelector<HTMLElement>(".cm-editor");
  const view = dom ? EditorView.findFromDOM(dom) : null;
  if (!view) {
    throw new Error("the text editor was not mounted");
  }
  return view;
}

export function replaceEditorText(scope: ParentNode, text: string): void {
  const view = textEditorView(scope);
  view.dispatch({
    changes: { from: 0, to: view.state.doc.length, insert: Text.of(text.split(/\r\n|\r|\n/)) },
    userEvent: "input.type",
  });
}

export function pressEditorKey(scope: ParentNode, key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  const view = textEditorView(scope);
  view.focus();
  const keyCode =
    ({ Tab: 9, Enter: 13, Escape: 27 } as Record<string, number>)[key] ??
    (key.length === 1 ? key.toUpperCase().charCodeAt(0) : 0);
  const pressed = init.shiftKey && /^[a-z]$/i.test(key) ? key.toUpperCase() : key;
  const event = new KeyboardEvent("keydown", { key: pressed, keyCode, bubbles: true, cancelable: true, ...init });
  view.contentDOM.dispatchEvent(event);
  return event;
}
