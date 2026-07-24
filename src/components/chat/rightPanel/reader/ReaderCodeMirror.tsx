import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { EditorState } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { useEffect, useMemo, useRef } from "react";
import { formatParagraphNumber } from "../../../../store/reader";
import type { TextRange } from "../../../../utils/readerFind";
import { diffSignGutterExtension } from "./readerCodeMirror/diffSignGutter";
import { applyFindHighlights, findHighlightField } from "./readerCodeMirror/findHighlights";
import { readerIndentKeymap } from "./readerCodeMirror/indentKeymap";
import {
  createReaderCodeMirrorTheme,
  type ReaderCodeMirrorLayout,
} from "./readerCodeMirror/theme";

export type ReaderCodeMirrorDiffVariant = "none" | "delete" | "insert";

export interface ReaderCodeMirrorProps {
  value: string;
  onChange?: (value: string) => void;
  ariaLabel: string;
  readOnly?: boolean;
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

export function ReaderCodeMirror({
  value,
  onChange,
  ariaLabel,
  readOnly = false,
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
  const onChangeRef = useRef(onChange);
  const syncingRef = useRef(false);
  const scrollToIndexRef = useRef(scrollToIndex);
  scrollToIndexRef.current = scrollToIndex;

  onChangeRef.current = onChange;

  const lineLabelsKey = useMemo(
    () => (lineLabels ? lineLabels.map((l) => l ?? "·").join(",") : ""),
    [lineLabels],
  );

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const labels = lineLabels;
    const formatLineNumber = labels
      ? (n: number) => {
          const label = labels[n - 1];
          return label != null ? formatParagraphNumber(label) : "";
        }
      : (n: number) => formatParagraphNumber(n);

    const editable = !readOnly && onChange != null;
    const showActiveLine = diffVariant !== "delete";

    const updateListener = EditorView.updateListener.of((update) => {
      if (!update.docChanged || syncingRef.current || !onChangeRef.current) return;
      onChangeRef.current(update.state.doc.toString());
    });

    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers({ formatNumber: formatLineNumber }),
        diffSignGutterExtension(diffSign, diffSignFirstLineOnly),
        EditorView.lineWrapping,
        ...(showActiveLine ? [highlightActiveLineGutter(), highlightActiveLine()] : []),
        drawSelection(),
        ...(editable ? [history()] : []),
        ...(layout === "document" ? [findHighlightField] : []),
        createReaderCodeMirrorTheme(layout),
        updateListener,
        ...(editable
          ? [keymap.of([...readerIndentKeymap, ...defaultKeymap, ...historyKeymap])]
          : []),
        EditorView.contentAttributes.of({ "aria-label": ariaLabel }),
        EditorView.editable.of(editable),
        EditorState.readOnly.of(!editable),
      ],
    });

    const view = new EditorView({ state, parent: host });
    viewRef.current = view;

    return () => {
      view.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- remount on structural props only
  }, [
    ariaLabel,
    readOnly,
    layout,
    diffVariant,
    diffSign,
    diffSignFirstLineOnly,
    lineLabelsKey,
    onChange != null,
  ]);

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current === value) return;
    syncingRef.current = true;
    view.dispatch({
      changes: { from: 0, to: current.length, insert: value },
    });
    syncingRef.current = false;
  }, [value]);

  useEffect(() => {
    if (layout !== "document") return;
    const view = viewRef.current;
    if (!view) return;
    applyFindHighlights(view, findRanges, findActiveIndex);
  }, [layout, findRanges, findActiveIndex]);

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
