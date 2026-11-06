import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { Compartment, EditorState } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { useContext, useEffect, useMemo, useRef } from "react";
import { useFileExplorer } from "../../../../store/fileExplorer";
import { formatParagraphNumber } from "../../../../store/reader";
import type { TextRange } from "../../../../utils/readerFind";
import {
  encodeReaderSelectionHtml,
  READER_SELECTION_MIME,
  rememberReaderSelection,
  type ReaderSelectionPayload,
} from "../../../../utils/readerSelection";
import { diffSignGutterExtension } from "./readerCodeMirror/diffSignGutter";
import { applyFindHighlights, findHighlightField } from "./readerCodeMirror/findHighlights";
import { readerIndentKeymap } from "./readerCodeMirror/indentKeymap";
import { ReaderEditorViewContext } from "./readerCodeMirror/editorViewContext";
import { runRegisteredReaderSave } from "./hooks/useEditorSave";
import {
  createReaderCodeMirrorTheme,
  type ReaderCodeMirrorLayout,
} from "./readerCodeMirror/theme";
import {
  readerViewportTop,
  rememberReaderViewport,
} from "../../../../utils/readerViewport";

export type ReaderCodeMirrorDiffVariant = "none" | "delete" | "insert";

export interface ReaderCodeMirrorProps {
  value: string;
  onChange?: (value: string) => void;
  ariaLabel: string;
  readOnly?: boolean;
  /** Absolute file path — enables multi-line copy → @file#P… paste into composer. */
  filePath?: string;
  /**
   * Added to 1-based document line numbers when computing paragraph range
   * (for segment editors that represent a slice of the full file).
   */
  paragraphBase?: number;
  /** Per-document-line paragraph numbers; null hides the label. */
  lineLabels?: (number | null)[];
  layout?: ReaderCodeMirrorLayout;
  diffVariant?: ReaderCodeMirrorDiffVariant;
  diffSign?: string;
  /** Show diff sign on the first line only (insert blocks). */
  diffSignFirstLineOnly?: boolean;
  findRanges?: TextRange[];
  findActiveIndex?: number;
  scrollToIndex?: number | null;
  scrollTrigger?: number;
}

function buildLineNumbers(labels: (number | null)[] | undefined) {
  return lineNumbers({
    formatNumber: labels
      ? (n: number) => {
          const label = labels[n - 1];
          return label != null ? formatParagraphNumber(label) : "";
        }
      : (n: number) => formatParagraphNumber(n),
  });
}

function selectionLineRange(
  view: EditorView,
): { fromLine: number; toLine: number; text: string } | null {
  const sel = view.state.selection.main;
  if (sel.empty) return null;
  const fromLine = view.state.doc.lineAt(sel.from).number;
  let toLine = view.state.doc.lineAt(sel.to).number;
  // Selection ending exactly at the start of a line does not include that line.
  if (sel.to > sel.from) {
    const lineAtTo = view.state.doc.lineAt(sel.to);
    if (sel.to === lineAtTo.from && toLine > fromLine) {
      toLine -= 1;
    }
  }
  return {
    fromLine,
    toLine: Math.max(fromLine, toLine),
    text: view.state.sliceDoc(sel.from, sel.to),
  };
}

export function ReaderCodeMirror({
  value,
  onChange,
  ariaLabel,
  readOnly = false,
  filePath,
  paragraphBase = 0,
  lineLabels,
  layout = "document",
  diffVariant = "none",
  diffSign,
  diffSignFirstLineOnly = false,
  findRanges = [],
  findActiveIndex = -1,
  scrollToIndex = null,
  scrollTrigger = -1,
}: ReaderCodeMirrorProps) {
  const hostRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const editorViewSlot = useContext(ReaderEditorViewContext);
  const editorViewSlotRef = useRef(editorViewSlot);
  editorViewSlotRef.current = editorViewSlot;
  const onChangeRef = useRef(onChange);
  const filePathRef = useRef(filePath);
  const paragraphBaseRef = useRef(paragraphBase);
  const syncingRef = useRef(false);
  const lastEmittedRef = useRef<string | null>(null);
  const scrollToIndexRef = useRef(scrollToIndex);
  scrollToIndexRef.current = scrollToIndex;
  filePathRef.current = filePath;
  paragraphBaseRef.current = paragraphBase;

  onChangeRef.current = onChange;

  const lineLabelsKey = useMemo(
    () => (lineLabels ? lineLabels.map((l) => l ?? "·").join(",") : ""),
    [lineLabels],
  );
  const lineLabelsRef = useRef(lineLabels);
  lineLabelsRef.current = lineLabels;
  const lineNumbersCompartmentRef = useRef<Compartment | null>(null);
  const appliedLabelsKeyRef = useRef(lineLabelsKey);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const lineNumbersCompartment = new Compartment();
    lineNumbersCompartmentRef.current = lineNumbersCompartment;
    appliedLabelsKeyRef.current = lineLabelsKey;

    const editable = !readOnly && onChange != null;
    const showActiveLine = diffVariant !== "delete";

    const updateListener = EditorView.updateListener.of((update) => {
      if (!update.docChanged || syncingRef.current || !onChangeRef.current) return;
      const next = update.state.doc.toString();
      lastEmittedRef.current = next;
      onChangeRef.current(next);
    });

    const copyHandler = EditorView.domEventHandlers({
      copy(event, view) {
        const path = filePathRef.current?.trim();
        if (!path) return false;
        const picked = selectionLineRange(view);
        if (!picked) return false;
        // Paragraphs here are whole (often very long) lines, so selecting one
        // paragraph is "one line". Only a selection that covers that entire
        // line becomes a card; a few words inside it stay plain text.
        if (picked.toLine <= picked.fromLine) {
          const sel = view.state.selection.main;
          const line = view.state.doc.lineAt(sel.from);
          const covers = sel.from <= line.from && sel.to >= line.to;
          if (!covers) return false;
        }
        const base = Math.max(0, paragraphBaseRef.current);
        const payload: ReaderSelectionPayload = {
          path,
          paragraphFrom: picked.fromLine + base,
          paragraphTo: picked.toLine + base,
          text: picked.text,
          projectRoot: useFileExplorer.getState().projectRoot,
          sessionId: useFileExplorer.getState().sessionId,
        };
        // Clipboard shows original text; composer turns this into an @ chip on paste.
        rememberReaderSelection(payload);
        event.clipboardData?.setData(READER_SELECTION_MIME, JSON.stringify(payload));
        event.clipboardData?.setData(
          "text/html",
          encodeReaderSelectionHtml(payload),
        );
        event.clipboardData?.setData("text/plain", picked.text);
        event.preventDefault();
        return true;
      },
    });

    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbersCompartment.of(buildLineNumbers(lineLabelsRef.current)),
        diffSignGutterExtension(diffSign, diffSignFirstLineOnly),
        EditorView.lineWrapping,
        ...(showActiveLine ? [highlightActiveLineGutter(), highlightActiveLine()] : []),
        drawSelection(),
        ...(editable ? [history()] : []),
        findHighlightField,
        createReaderCodeMirrorTheme(layout),
        updateListener,
        copyHandler,
        ...(editable
          ? [keymap.of([
              {
                key: "Mod-s",
                preventDefault: true,
                run: runRegisteredReaderSave,
              },
              ...readerIndentKeymap,
              ...defaultKeymap,
              ...historyKeymap,
            ])]
          : []),
        EditorView.contentAttributes.of({ "aria-label": ariaLabel }),
        EditorView.editable.of(editable),
        EditorState.readOnly.of(!editable),
      ],
    });

    const view = new EditorView({ state, parent: host });
    viewRef.current = view;
    const slot = editorViewSlotRef.current;
    if (slot && layout === "document" && editable) slot.current = view;

    const destroy = () => {
      if (slot?.current === view) slot.current = null;
      view.destroy();
      viewRef.current = null;
    };

    const scrollEl = view.scrollDOM;
    if (layout === "document" && filePath) {
      scrollEl.scrollTop = readerViewportTop(filePath);
      const onScroll = () => {
        const path = filePathRef.current?.trim() || filePath;
        rememberReaderViewport(path, scrollEl.scrollTop);
      };
      scrollEl.addEventListener("scroll", onScroll, { passive: true });
      return () => {
        scrollEl.removeEventListener("scroll", onScroll);
        try {
          onScroll();
        } finally {
          destroy();
        }
      };
    }

    return destroy;
    // eslint-disable-next-line react-hooks/exhaustive-deps -- remount on structural props only
  }, [
    ariaLabel,
    readOnly,
    layout,
    diffVariant,
    diffSign,
    diffSignFirstLineOnly,
    onChange != null,
  ]);

  // Paragraph labels shift whenever lines are added/removed above this segment.
  // Reconfigure the gutter in place instead of tearing the whole editor down.
  useEffect(() => {
    if (appliedLabelsKeyRef.current === lineLabelsKey) return;
    appliedLabelsKeyRef.current = lineLabelsKey;
    const view = viewRef.current;
    const compartment = lineNumbersCompartmentRef.current;
    if (!view || !compartment) return;
    view.dispatch({
      effects: compartment.reconfigure(buildLineNumbers(lineLabelsRef.current)),
    });
  }, [lineLabelsKey]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current === value) {
      lastEmittedRef.current = null;
      return;
    }
    // Segment editors: while the user is typing here, the editor is the source of
    // truth. A parent that momentarily derives a different slice (e.g. line range
    // drift after Enter) must not overwrite it — that would drop the new line and
    // the caret.
    if (layout === "segment" && view.hasFocus && current === lastEmittedRef.current) {
      return;
    }
    const top = view.scrollDOM.scrollTop;
    syncingRef.current = true;
    view.dispatch({
      changes: { from: 0, to: current.length, insert: value },
    });
    syncingRef.current = false;
    view.scrollDOM.scrollTop = top;
    if (layout === "document" && filePath) {
      rememberReaderViewport(filePath, view.scrollDOM.scrollTop);
    }
  }, [value, layout, filePath]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    applyFindHighlights(view, findRanges, findActiveIndex);
  }, [findRanges, findActiveIndex]);

  // Scroll only when the user navigates find matches (scrollTrigger), not on every
  // edit that shifts match offsets in the live document.
  useEffect(() => {
    if (layout !== "document") return;
    const view = viewRef.current;
    const target = scrollToIndexRef.current;
    if (!view || target == null) return;
    const docLen = view.state.doc.length;
    const pos = Math.max(0, Math.min(target, docLen));
    view.dispatch({
      effects: EditorView.scrollIntoView(pos, { y: "nearest" }),
    });
  }, [layout, scrollTrigger]);

  const hostClass = [
    "reader-codemirror",
    layout === "segment" ? "reader-codemirror--segment" : "",
    diffVariant !== "none" ? `reader-codemirror--${diffVariant}` : "",
  ]
    .filter(Boolean)
    .join(" ");

  return <div ref={hostRef} className={hostClass} />;
}
