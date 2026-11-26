import { indentMore } from "@codemirror/commands";
import { indentUnit } from "@codemirror/language";
import { EditorSelection, type StateCommand } from "@codemirror/state";

/** A selection is indented by line; an empty selection inserts at the actual cursor. */
export const insertIndent: StateCommand = ({ state, dispatch }) => {
  if (state.readOnly) {
    return false;
  }
  if (state.selection.ranges.some((range) => !range.empty)) {
    return indentMore({ state, dispatch });
  }
  const unit = state.facet(indentUnit);
  dispatch(
    state.update(
      state.changeByRange((range) => ({
        changes: { from: range.from, insert: unit },
        range: EditorSelection.cursor(range.from + unit.length),
      })),
      { scrollIntoView: true, userEvent: "input.indent" },
    ),
  );
  return true;
};

/** YAML may already contain pasted Tabs; new indentation must still be written as spaces. */
export const insertYamlNewline: StateCommand = ({ state, dispatch }) => {
  if (state.readOnly) {
    return false;
  }
  dispatch(
    state.update(
      state.changeByRange((range) => {
        const leading = /^[ \t]*/.exec(state.doc.lineAt(range.from).text)?.[0] ?? "";
        let columns = 0;
        for (const character of leading) {
          columns += character === "\t" ? state.tabSize - (columns % state.tabSize) : 1;
        }
        const indentation = " ".repeat(columns);
        return {
          changes: { from: range.from, to: range.to, insert: `\n${indentation}` },
          range: EditorSelection.cursor(range.from + 1 + indentation.length),
        };
      }),
      { scrollIntoView: true, userEvent: "input" },
    ),
  );
  return true;
};

/** The editor's document uses logical lines, but saving keeps the file's original line separator. */
export function lineSeparatorOf(text: string): string {
  return /\r\n|\r|\n/.exec(text)?.[0] ?? "\n";
}
