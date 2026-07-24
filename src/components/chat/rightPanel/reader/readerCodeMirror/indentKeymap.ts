import { indentLess, indentMore, insertNewline } from "@codemirror/commands";
import type { KeyBinding } from "@codemirror/view";
import type { EditorView } from "@codemirror/view";
import { buildParagraphIndentChanges } from "../../../../../utils/readerIndent";

function runParagraphIndent(view: EditorView, outdent: boolean): boolean {
  const text = view.state.doc.toString();
  const sel = view.state.selection.main;
  const result = buildParagraphIndentChanges(text, sel.from, sel.to, outdent);
  if (!result) return false;
  view.dispatch({
    changes: result.changes,
    selection: { anchor: result.selectionStart, head: result.selectionEnd },
    // Keep the viewport stable while indenting (especially with a leading space).
    scrollIntoView: false,
  });
  return true;
}

export const readerIndentKeymap: KeyBinding[] = [
  {
    // Default CM Enter copies leading whitespace (insertNewlineAndIndent). Prose
    // uses Tab for 首行缩进 only �?new lines should start flush left.
    key: "Enter",
    run: insertNewline,
  },
  {
    key: "Tab",
    run(view) {
      if (runParagraphIndent(view, false)) return true;
      return indentMore(view);
    },
  },
  {
    key: "Shift-Tab",
    run(view) {
      if (runParagraphIndent(view, true)) return true;
      return indentLess(view);
    },
  },
];
