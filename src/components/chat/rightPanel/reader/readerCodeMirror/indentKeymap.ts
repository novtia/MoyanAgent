import { indentLess, indentMore, insertNewline } from "@codemirror/commands";
import type { KeyBinding } from "@codemirror/view";
import type { EditorView } from "@codemirror/view";
import {
  buildDocumentFirstLineIndentChanges,
  buildParagraphIndentChanges,
  type DocumentFormatOptions,
} from "../../../../../utils/readerIndent";

function dispatchIndent(
  view: EditorView,
  result: {
    changes: { from: number; to: number; insert: string }[];
    selectionStart: number;
    selectionEnd: number;
  } | null,
): boolean {
  if (!result) return false;
  view.dispatch({
    changes: result.changes,
    selection: { anchor: result.selectionStart, head: result.selectionEnd },
    // Keep the viewport stable while indenting (especially with a leading space).
    scrollIntoView: false,
  });
  return true;
}

function runParagraphIndent(view: EditorView, outdent: boolean): boolean {
  const text = view.state.doc.toString();
  const sel = view.state.selection.main;
  return dispatchIndent(
    view,
    buildParagraphIndentChanges(text, sel.from, sel.to, outdent),
  );
}

export function runDocumentFirstLineIndent(
  view: EditorView,
  outdent: boolean,
  markdown: boolean,
  options: DocumentFormatOptions,
): boolean {
  const text = view.state.doc.toString();
  const sel = view.state.selection.main;
  return dispatchIndent(
    view,
    buildDocumentFirstLineIndentChanges(
      text,
      outdent,
      markdown,
      sel.from,
      sel.to,
      options,
    ),
  );
}

/** Toolbar format for the editor owned by this reader pane. */
export function indentReaderView(
  view: EditorView | null,
  outdent: boolean,
  markdown: boolean,
  options: DocumentFormatOptions,
): boolean {
  if (!view || view.state.readOnly) return false;
  const ok = runDocumentFirstLineIndent(view, outdent, markdown, options);
  if (ok && view.dom.isConnected) view.focus();
  return ok;
}

export const readerIndentKeymap: KeyBinding[] = [
  {
    // Default CM Enter copies leading whitespace (insertNewlineAndIndent). Prose
    // uses Tab for first-line indent only. New lines should start flush left.
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
