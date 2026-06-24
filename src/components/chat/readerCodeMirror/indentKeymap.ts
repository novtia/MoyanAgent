import { indentLess, indentMore } from "@codemirror/commands";
import type { KeyBinding } from "@codemirror/view";
import type { EditorView } from "@codemirror/view";
import { applyParagraphIndent } from "../../../utils/readerIndent";

function runParagraphIndent(view: EditorView, outdent: boolean): boolean {
  const text = view.state.doc.toString();
  const sel = view.state.selection.main;
  const result = applyParagraphIndent(text, sel.from, sel.to, outdent);
  if (!result) return false;
  view.dispatch({
    changes: { from: 0, to: text.length, insert: result.text },
    selection: { anchor: result.selectionStart, head: result.selectionEnd },
  });
  return true;
}

export const readerIndentKeymap: KeyBinding[] = [
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
