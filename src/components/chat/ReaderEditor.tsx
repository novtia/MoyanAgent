import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type RefObject,
} from "react";
import { useSession } from "../../store/session";
import {
  buildLineParagraphLabels,
  formatParagraphNumber,
  paragraphAt,
  useReader,
  type ReaderFileTab,
  type ReaderPendingDiff,
} from "../../store/reader";
import { api } from "../../api/tauri";
import {
  buildEditorDisplaySegments,
  buildPendingDiffLineRanges,
  replaceTabLineRange,
  sliceTabLines,
  type EditorDisplaySegment,
} from "../../utils/inlineDiff";
import {
  measureWrappedLineHeights,
  textareaContentWidth,
} from "../../utils/readerGutter";
import { handleReaderIndentKeyDown } from "../../utils/readerIndent";
import { ReaderDiffActionBar } from "./ReaderDiffActionBar";

const SAVE_DEBOUNCE_MS = 600;

function usePendingSelection(
  ref: RefObject<HTMLTextAreaElement | null>,
  contentKey: string,
) {
  const pending = useRef<[number, number] | null>(null);

  const queueSelection = useCallback((start: number, end: number) => {
    pending.current = [start, end];
  }, []);

  useLayoutEffect(() => {
    const pendingSel = pending.current;
    if (!pendingSel || !ref.current) return;
    pending.current = null;
    ref.current.setSelectionRange(pendingSel[0], pendingSel[1]);
  }, [contentKey, ref]);

  return queueSelection;
}

function onIndentKeyDown(
  e: KeyboardEvent<HTMLTextAreaElement>,
  text: string,
  apply: (next: string, selStart: number, selEnd: number) => void,
) {
  const result = handleReaderIndentKeyDown(e, text);
  if (!result) return;
  e.preventDefault();
  apply(result.text, result.selectionStart, result.selectionEnd);
}

interface ReaderEditorProps {
  tab: ReaderFileTab;
}

function resolveDiffBlock(block: ReaderPendingDiff): ReaderPendingDiff {
  let before = block.before;
  let after = block.after;
  if (!before.trim() && !after.trim() && block.paragraphNumber != null) {
    before = paragraphAt(block.textBefore, block.paragraphNumber);
    after = paragraphAt(block.textAfter, block.paragraphNumber);
  }
  return { ...block, before, after };
}

function GutterStack({
  labels,
  heights,
}: {
  labels: (number | null)[];
  heights: number[];
}) {
  return (
    <>
      {labels.map((label, i) => (
        <div
          key={i}
          className="reader-editor-gutter-item"
          style={{ height: heights[i] > 0 ? heights[i] : undefined }}
        >
          {label != null ? formatParagraphNumber(label) : ""}
        </div>
      ))}
    </>
  );
}

function useLineHeights(
  lines: string[],
  contentWidth: number,
  styleRef: RefObject<HTMLElement | null>,
  contentKey: string,
): number[] {
  const [heights, setHeights] = useState<number[]>(() => lines.map(() => 0));

  useLayoutEffect(() => {
    const el = styleRef.current;
    if (!el || contentWidth <= 0) {
      setHeights((prev) => {
        const zeros = lines.map(() => 0);
        if (prev.length === zeros.length && prev.every((h) => h === 0)) return prev;
        return zeros;
      });
      return;
    }
    const measured = measureWrappedLineHeights(lines, contentWidth, el);
    setHeights((prev) => {
      if (prev.length === measured.length && prev.every((h, i) => h === measured[i])) {
        return prev;
      }
      return measured;
    });
  }, [contentKey, contentWidth, styleRef]);

  return heights;
}

function SegmentBlock({
  labels,
  lines,
  value,
  onChange,
  className,
  readOnly,
  style,
  ariaLabel,
  sign,
}: {
  labels: (number | null)[];
  lines: string[];
  value: string;
  onChange?: (value: string) => void;
  className?: string;
  readOnly?: boolean;
  style?: CSSProperties;
  ariaLabel?: string;
  sign?: string;
}) {
  const fieldRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);
  const queueSelection = usePendingSelection(fieldRef, value);
  const [contentWidth, setContentWidth] = useState(0);

  useLayoutEffect(() => {
    const el = fieldRef.current;
    if (!el) return;
    const update = () => {
      const next = textareaContentWidth(el);
      setContentWidth((prev) => (prev === next ? prev : next));
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    const wrap = el.closest(".reader-editor-wrap");
    if (wrap) ro.observe(wrap);
    return () => ro.disconnect();
  }, [value]);

  const heights = useLineHeights(lines, contentWidth, fieldRef, value);

  const syncScroll = useCallback(() => {
    if (gutterRef.current && fieldRef.current) {
      gutterRef.current.scrollTop = fieldRef.current.scrollTop;
    }
  }, []);

  const totalHeight = heights.reduce((sum, h) => sum + (h > 0 ? h : 0), 0);

  return (
    <div
      className={`reader-editor-block${sign ? " is-signed" : ""}${className?.includes("is-insert-field") ? " is-insert-block" : ""}`}
      style={style}
    >
      <div ref={gutterRef} className="reader-editor-gutter">
        <GutterStack labels={labels} heights={heights} />
      </div>
      {sign ? (
        <span className="reader-editor-line-sign" aria-hidden>
          {sign}
        </span>
      ) : null}
      <textarea
        ref={fieldRef}
        className={className ?? "reader-editor-field"}
        value={value}
        readOnly={readOnly}
        onChange={onChange ? (e) => onChange(e.target.value) : undefined}
        onKeyDown={
          onChange
            ? (e) =>
                onIndentKeyDown(e, value, (next, selStart, selEnd) => {
                  onChange(next);
                  queueSelection(selStart, selEnd);
                })
            : undefined
        }
        onScroll={syncScroll}
        spellCheck={false}
        aria-label={ariaLabel}
        style={totalHeight > 0 ? { height: totalHeight } : undefined}
      />
    </div>
  );
}

function PlainReader({
  tab,
  applyText,
}: {
  tab: ReaderFileTab;
  applyText: (text: string) => void;
}) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const gutterRef = useRef<HTMLDivElement>(null);
  const queueSelection = usePendingSelection(textareaRef, tab.text);
  const lines = useMemo(() => tab.text.split("\n"), [tab.text]);
  const labels = useMemo(() => buildLineParagraphLabels(tab.text), [tab.text]);
  const [contentWidth, setContentWidth] = useState(0);

  useLayoutEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    const update = () => {
      const next = textareaContentWidth(el);
      setContentWidth((prev) => (prev === next ? prev : next));
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    const wrap = el.closest(".reader-editor-wrap");
    if (wrap) ro.observe(wrap);
    return () => ro.disconnect();
  }, [tab.text]);

  const heights = useLineHeights(lines, contentWidth, textareaRef, tab.text);

  const syncScroll = useCallback(() => {
    if (gutterRef.current && textareaRef.current) {
      gutterRef.current.scrollTop = textareaRef.current.scrollTop;
    }
  }, []);

  return (
    <div className="reader-editor-wrap reader-editor-wrap--plain">
      <div className="reader-editor-plain">
        <div ref={gutterRef} className="reader-editor-gutter">
          <GutterStack labels={labels} heights={heights} />
        </div>
        <textarea
          ref={textareaRef}
          className="reader-editor"
          value={tab.text}
          onChange={(e) => applyText(e.target.value)}
          onKeyDown={(e) =>
            onIndentKeyDown(e, tab.text, (next, selStart, selEnd) => {
              applyText(next);
              queueSelection(selStart, selEnd);
            })
          }
          onScroll={syncScroll}
          spellCheck={false}
          aria-label={tab.path}
        />
      </div>
    </div>
  );
}

export function ReaderEditor({ tab }: ReaderEditorProps) {
  const sessionId = useSession((s) => s.activeId);
  const updateTabText = useReader((s) => s.updateTabText);
  const setTabDirty = useReader((s) => s.setTabDirty);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latestTextRef = useRef(tab.text);
  const dirtyRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const mainRef = useRef<HTMLDivElement>(null);
  const hunkRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const [hoveredBlockId, setHoveredBlockId] = useState<string | null>(null);
  const hideBarTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const hasPendingDiff = tab.pendingDiffs.length > 0;
  const paraLabels = useMemo(() => buildLineParagraphLabels(tab.text), [tab.text]);
  const fileLines = useMemo(() => tab.text.split("\n"), [tab.text]);

  const diffBlocks = useMemo(
    () =>
      tab.pendingDiffs.map((d) => {
        const s = resolveDiffBlock(d);
        return {
          id: s.id,
          before: s.before,
          after: s.after,
          textBefore: s.textBefore,
          textAfter: s.textAfter,
          paragraphNumber: s.paragraphNumber,
        };
      }),
    [tab.pendingDiffs],
  );

  const lineRanges = useMemo(
    () => (hasPendingDiff ? buildPendingDiffLineRanges(tab.text, diffBlocks) : []),
    [tab.text, diffBlocks, hasPendingDiff],
  );

  const displaySegments = useMemo(
    () => (hasPendingDiff ? buildEditorDisplaySegments(tab.text, diffBlocks) : []),
    [tab.text, diffBlocks, hasPendingDiff],
  );

  const hoveredRange = useMemo(
    () => lineRanges.find((r) => r.blockId === hoveredBlockId) ?? null,
    [lineRanges, hoveredBlockId],
  );

  const hoveredHunkIndex = hoveredRange
    ? lineRanges.findIndex((r) => r.blockId === hoveredRange.blockId)
    : -1;

  useEffect(() => {
    latestTextRef.current = tab.text;
    if (!tab.dirty) dirtyRef.current = false;
  }, [tab.text, tab.dirty]);

  const flushSave = useCallback(
    async (text: string) => {
      if (!sessionId || !tab.path) return;
      try {
        await api.writeProjectFile(sessionId, tab.path, text);
        dirtyRef.current = false;
        setTabDirty(tab.path, false, false);
      } catch {
        setTabDirty(tab.path, true, true);
      }
    },
    [sessionId, tab.path, setTabDirty],
  );

  const scheduleSave = useCallback(
    (text: string) => {
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        void flushSave(text);
      }, SAVE_DEBOUNCE_MS);
    },
    [flushSave],
  );

  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      if (dirtyRef.current && sessionId && tab.path) {
        void api.writeProjectFile(sessionId, tab.path, latestTextRef.current).catch(() => {
          setTabDirty(tab.path, true, true);
        });
      }
    };
  }, [sessionId, tab.path, setTabDirty]);

  const applyText = useCallback(
    (text: string) => {
      latestTextRef.current = text;
      dirtyRef.current = true;
      updateTabText(tab.path, text, { dirty: true });
      scheduleSave(text);
    },
    [tab.path, updateTabText, scheduleSave],
  );

  const onSegmentChange = useCallback(
    (tabStart: number, tabEnd: number, value: string) => {
      applyText(replaceTabLineRange(tab.text, tabStart, tabEnd, value));
    },
    [applyText, tab.text],
  );

  const showBarForBlock = useCallback((blockId: string | null) => {
    if (hideBarTimerRef.current) {
      clearTimeout(hideBarTimerRef.current);
      hideBarTimerRef.current = null;
    }
    setHoveredBlockId(blockId);
  }, []);

  const scheduleHideBar = useCallback(() => {
    if (hideBarTimerRef.current) clearTimeout(hideBarTimerRef.current);
    hideBarTimerRef.current = setTimeout(() => setHoveredBlockId(null), 220);
  }, []);

  const navigateHunk = useCallback(
    (direction: -1 | 1) => {
      if (hoveredHunkIndex < 0) return;
      const next = lineRanges[hoveredHunkIndex + direction];
      if (!next) return;
      showBarForBlock(next.blockId);
      hunkRefs.current.get(next.blockId)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    },
    [hoveredHunkIndex, lineRanges, showBarForBlock],
  );

  const renderContextBlock = (seg: Extract<EditorDisplaySegment, { kind: "context" }>) => {
    const segmentLines: string[] = [];
    const segmentLabels: (number | null)[] = [];
    for (let i = seg.tabStart; i <= seg.tabEnd; i += 1) {
      segmentLines.push(fileLines[i] ?? "");
      segmentLabels.push(paraLabels[i] ?? null);
    }
    return (
      <SegmentBlock
        key={`ctx-${seg.tabStart}-${seg.tabEnd}`}
        labels={segmentLabels}
        lines={segmentLines}
        value={sliceTabLines(tab.text, seg.tabStart, seg.tabEnd)}
        onChange={(value) => onSegmentChange(seg.tabStart, seg.tabEnd, value)}
        className="reader-editor-field reader-editor-segment"
        ariaLabel={tab.path}
      />
    );
  };

  const renderHunk = (seg: Extract<EditorDisplaySegment, { kind: "hunk" }>) => {
    const deleteLines = seg.before.trim() ? seg.before.split("\n") : [];
    const insertLines: string[] = [];
    const insertLabels: (number | null)[] = [];
    for (let i = seg.tabStart; i <= seg.tabEnd; i += 1) {
      insertLines.push(fileLines[i] ?? "");
      insertLabels.push(paraLabels[i] ?? null);
    }

    return (
      <div
        key={seg.blockId}
        ref={(el) => {
          if (el) hunkRefs.current.set(seg.blockId, el);
          else hunkRefs.current.delete(seg.blockId);
        }}
        className="reader-editor-hunk"
        onMouseEnter={() => showBarForBlock(seg.blockId)}
        onMouseLeave={scheduleHideBar}
      >
        {deleteLines.map((line, i) => (
          <DeleteLineRow
            key={`del-${i}`}
            label={paraLabels[seg.tabStart + i] ?? null}
            line={line}
          />
        ))}
        {insertLines.length > 0 && (
          <SegmentBlock
            labels={insertLabels}
            lines={insertLines}
            value={sliceTabLines(tab.text, seg.tabStart, seg.tabEnd)}
            onChange={(value) => onSegmentChange(seg.tabStart, seg.tabEnd, value)}
            className="reader-editor-field is-insert-field"
            sign="+"
            ariaLabel={tab.path}
          />
        )}
      </div>
    );
  };

  if (!hasPendingDiff) {
    return <PlainReader tab={tab} applyText={applyText} />;
  }

  const anchorEl = hoveredBlockId ? hunkRefs.current.get(hoveredBlockId) ?? null : null;

  return (
    <div className="reader-editor-wrap reader-editor-wrap--diff">
      <div ref={mainRef} className="reader-editor-main" onMouseLeave={scheduleHideBar}>
        <div ref={scrollRef} className="reader-editor-scroll">
          {displaySegments.flatMap((seg) =>
            seg.kind === "context"
              ? [renderContextBlock(seg)]
              : [renderHunk(seg)],
          )}
        </div>
        {hoveredRange && hoveredHunkIndex >= 0 && (
          <ReaderDiffActionBar
            tab={tab}
            range={hoveredRange}
            hunkIndex={hoveredHunkIndex}
            hunkTotal={lineRanges.length}
            anchorEl={anchorEl}
            mainRef={mainRef}
            onNavigate={navigateHunk}
            onMouseEnter={() => showBarForBlock(hoveredRange.blockId)}
            onMouseLeave={scheduleHideBar}
          />
        )}
      </div>
    </div>
  );
}

/** Read-only deleted line with gutter + diff styling. */
function DeleteLineRow({ label, line }: { label: number | null; line: string }) {
  const textRef = useRef<HTMLSpanElement>(null);
  const [contentWidth, setContentWidth] = useState(0);

  useLayoutEffect(() => {
    const el = textRef.current;
    if (!el) return;
    const update = () => {
      const next = el.clientWidth;
      setContentWidth((prev) => (prev === next ? prev : next));
    };
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    const wrap = el.closest(".reader-editor-wrap");
    if (wrap) ro.observe(wrap);
    return () => ro.disconnect();
  }, [line]);

  const heights = useLineHeights([line], contentWidth, textRef, line);

  return (
    <div className="reader-editor-row reader-editor-line is-delete">
      <div className="reader-editor-gutter reader-editor-gutter--inline">
        <div
          className="reader-editor-gutter-item"
          style={{ height: heights[0] > 0 ? heights[0] : undefined }}
        >
          {label != null ? formatParagraphNumber(label) : ""}
        </div>
      </div>
      <span className="reader-editor-line-sign" aria-hidden>
        −
      </span>
      <span ref={textRef} className="reader-editor-line-text">
        {line || "\u00a0"}
      </span>
    </div>
  );
}
