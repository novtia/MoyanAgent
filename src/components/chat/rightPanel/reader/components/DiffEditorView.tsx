import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  buildLineParagraphLabels,
  type ReaderFileTab,
} from "../../../../../store/reader";
import {
  buildEditorDisplaySegments,
  buildPendingDiffLineRanges,
  isDiffTextEqual,
  replaceTabLineRange,
  sliceTabLines,
  type EditorDisplaySegment,
} from "../../../../../utils/inlineDiff";
import { DIFF_BAR_HIDE_MS } from "../constants";
import { ReaderCodeMirror } from "../ReaderCodeMirror";
import { ReaderDiffActionBar } from "../ReaderDiffActionBar";

export function DiffEditorView({
  tab,
  applyText,
  activeHunkIndex,
  onActiveHunkChange,
}: {
  tab: ReaderFileTab;
  applyText: (text: string) => void;
  activeHunkIndex?: number;
  onActiveHunkChange?: (index: number) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const hunkRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const [hoveredBlockId, setHoveredBlockId] = useState<string | null>(null);
  const hideBarTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const paraLabels = useMemo(() => buildLineParagraphLabels(tab.text), [tab.text]);

  const diffBlocks = useMemo(
    () =>
      tab.pendingDiffs.map((d) => ({
        id: d.id,
        before: d.before,
        after: d.after,
        textBefore: d.textBefore,
        textAfter: d.textAfter,
        paragraphNumber: d.paragraphNumber,
      })),
    [tab.pendingDiffs],
  );

  const lineRanges = useMemo(
    () => buildPendingDiffLineRanges(tab.text, diffBlocks),
    [tab.text, diffBlocks],
  );

  const displaySegments = useMemo(
    () => buildEditorDisplaySegments(tab.text, diffBlocks),
    [tab.text, diffBlocks],
  );

  const hoveredHunkIndex = useMemo(
    () =>
      hoveredBlockId
        ? lineRanges.findIndex((r) => r.blockId === hoveredBlockId)
        : -1,
    [hoveredBlockId, lineRanges],
  );

  const onSegmentChange = useCallback(
    (tabStart: number, tabEnd: number, value: string) => {
      applyText(replaceTabLineRange(tab.text, tabStart, tabEnd, value));
    },
    [applyText, tab.text],
  );

  const showBarForBlock = useCallback(
    (blockId: string | null) => {
      if (hideBarTimerRef.current) {
        clearTimeout(hideBarTimerRef.current);
        hideBarTimerRef.current = null;
      }
      setHoveredBlockId(blockId);
      if (blockId && onActiveHunkChange) {
        const idx = lineRanges.findIndex((r) => r.blockId === blockId);
        if (idx >= 0) onActiveHunkChange(idx);
      }
    },
    [lineRanges, onActiveHunkChange],
  );

  const scheduleHideBar = useCallback(() => {
    if (hideBarTimerRef.current) clearTimeout(hideBarTimerRef.current);
    hideBarTimerRef.current = setTimeout(() => setHoveredBlockId(null), DIFF_BAR_HIDE_MS);
  }, []);

  const navigateHunk = useCallback(
    (direction: -1 | 1) => {
      const base =
        hoveredHunkIndex >= 0
          ? hoveredHunkIndex
          : activeHunkIndex != null && activeHunkIndex >= 0
            ? activeHunkIndex
            : 0;
      const nextIdx = base + direction;
      const next = lineRanges[nextIdx];
      if (!next) return;
      showBarForBlock(next.blockId);
      onActiveHunkChange?.(nextIdx);
      hunkRefs.current.get(next.blockId)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    },
    [hoveredHunkIndex, activeHunkIndex, lineRanges, showBarForBlock, onActiveHunkChange],
  );

  useEffect(() => {
    if (activeHunkIndex == null || activeHunkIndex < 0) return;
    const range = lineRanges[activeHunkIndex];
    if (!range) return;
    if (hideBarTimerRef.current) {
      clearTimeout(hideBarTimerRef.current);
      hideBarTimerRef.current = null;
    }
    setHoveredBlockId(range.blockId);
    hunkRefs.current.get(range.blockId)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [activeHunkIndex, lineRanges]);

  const renderContextBlock = (seg: Extract<EditorDisplaySegment, { kind: "context" }>) => {
    const segmentLabels: (number | null)[] = [];
    for (let i = seg.tabStart; i <= seg.tabEnd; i += 1) {
      segmentLabels.push(paraLabels[i] ?? null);
    }
    return (
      <ReaderCodeMirror
        key={`ctx-${seg.tabStart}-${seg.tabEnd}`}
        layout="segment"
        lineLabels={segmentLabels}
        value={sliceTabLines(tab.text, seg.tabStart, seg.tabEnd)}
        onChange={(value) => onSegmentChange(seg.tabStart, seg.tabEnd, value)}
        ariaLabel={tab.path}
      />
    );
  };

  const renderHunk = (seg: Extract<EditorDisplaySegment, { kind: "hunk" }>) => {
    if (isDiffTextEqual(seg.before, seg.after)) {
      return renderContextBlock({
        kind: "context",
        tabStart: seg.tabStart,
        tabEnd: seg.tabEnd,
      });
    }

    const deleteLines = seg.before.trim() ? seg.before.split("\n") : [];
    const insertLines = seg.after ? seg.after.split("\n") : [];
    const insertLabels: (number | null)[] = insertLines.map((_, i) => {
      if (seg.paragraphNumber == null) return null;
      return seg.before.trim() === "" && seg.after.trim() !== ""
        ? seg.paragraphNumber + 1 + i
        : seg.paragraphNumber + i;
    });
    const range = lineRanges.find((r) => r.blockId === seg.blockId);
    const hunkIndex = range ? lineRanges.findIndex((r) => r.blockId === seg.blockId) : -1;
    const showBar = hoveredBlockId === seg.blockId && range != null && hunkIndex >= 0;

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
          <ReaderCodeMirror
            key={`del-${seg.blockId}-${i}`}
            layout="segment"
            diffVariant="delete"
            diffSign="−"
            lineLabels={[paraLabels[seg.tabStart + i] ?? null]}
            value={line}
            readOnly
            ariaLabel="removed line"
          />
        ))}
        {insertLines.length > 0 && (
          <ReaderCodeMirror
            key={`ins-${seg.blockId}`}
            layout="segment"
            diffVariant="insert"
            diffSign="+"
            diffSignFirstLineOnly
            lineLabels={insertLabels}
            value={seg.after}
            onChange={(value) => onSegmentChange(seg.tabStart, seg.tabEnd, value)}
            ariaLabel={tab.path}
          />
        )}
        {showBar && (
          <ReaderDiffActionBar
            tab={tab}
            range={range}
            hunkIndex={hunkIndex}
            hunkTotal={lineRanges.length}
            onNavigate={navigateHunk}
            onMouseEnter={() => showBarForBlock(seg.blockId)}
            onMouseLeave={scheduleHideBar}
          />
        )}
      </div>
    );
  };

  return (
    <div className="reader-editor-wrap reader-editor-wrap--diff reader-editor-wrap--codemirror">
      <div className="reader-editor-main" onMouseLeave={scheduleHideBar}>
        <div ref={scrollRef} className="reader-editor-scroll">
          {displaySegments.flatMap((seg) =>
            seg.kind === "context"
              ? [renderContextBlock(seg)]
              : [renderHunk(seg)],
          )}
        </div>
      </div>
    </div>
  );
}
