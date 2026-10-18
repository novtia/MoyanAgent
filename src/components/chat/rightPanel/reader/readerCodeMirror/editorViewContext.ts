import { createContext, type MutableRefObject } from "react";
import type { EditorView } from "@codemirror/view";

/**
 * The toolbar and the source editor of one reader pane.
 * Hidden panes stay mounted, so this must not be a module-wide singleton.
 */
export const ReaderEditorViewContext =
  createContext<MutableRefObject<EditorView | null> | null>(null);
