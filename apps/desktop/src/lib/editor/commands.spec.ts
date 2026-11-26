import { indentUnit } from "@codemirror/language";
import { EditorSelection, EditorState } from "@codemirror/state";
import { describe, expect, it } from "vitest";

import { insertIndent, lineSeparatorOf } from "@/lib/editor/commands";

describe("inserting an indentation transaction", () => {
  it("inserts spaces at the cursor instead of at the beginning of the line", () => {
    let state = EditorState.create({ doc: "ab", selection: { anchor: 1 }, extensions: indentUnit.of("    ") });
    insertIndent({
      state,
      dispatch: (transaction) => {
        state = transaction.state;
      },
    });
    expect(state.doc.toString()).toBe("a    b");
    expect(state.selection.main.head).toBe(5);
    expect(state.selection.main.empty).toBe(true);
  });

  it("inserts one literal Tab when that is the configured indentation", () => {
    let state = EditorState.create({ doc: "ab", selection: { anchor: 1 }, extensions: indentUnit.of("\t") });
    insertIndent({
      state,
      dispatch: (transaction) => {
        state = transaction.state;
      },
    });
    expect(state.doc.toString()).toBe("a\tb");
    expect(state.selection.main.head).toBe(2);
  });

  it("indents selected lines without touching a line selected only at its first column", () => {
    let state = EditorState.create({
      doc: "a\nb\nc",
      selection: EditorSelection.range(0, 4),
      extensions: indentUnit.of("  "),
    });
    insertIndent({
      state,
      dispatch: (transaction) => {
        state = transaction.state;
      },
    });
    expect(state.doc.toString()).toBe("  a\n  b\nc");
    expect(state.selection.main.empty).toBe(false);
  });

  it("does not turn a read-only document into an editable one", () => {
    const state = EditorState.create({ doc: "keep", extensions: EditorState.readOnly.of(true) });
    let dispatched = false;
    expect(
      insertIndent({
        state,
        dispatch: () => {
          dispatched = true;
        },
      }),
    ).toBe(false);
    expect(dispatched).toBe(false);
  });
});

describe("the line separator used for saving", () => {
  it.each([
    ["a\r\nb\r\n", "\r\n"],
    ["a\nb\n", "\n"],
    ["a\rb\r", "\r"],
    ["a", "\n"],
  ])("keeps the separator of %j", (text, expected) => {
    expect(lineSeparatorOf(text)).toBe(expected);
  });
});
