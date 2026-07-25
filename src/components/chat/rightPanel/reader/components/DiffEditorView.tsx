import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
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
  const scrollTopRef = useRef(0);
  const lastScrolledHunkRef = useRef<number | null>(null);
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
    lastScrolledHunkRef.current = null;
    scrollTopRef.current = 0;
  }, [tab.path]);

  // Track scroll so confirming a hunk (DOM rebuild) does not jump to top.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const onScroll = () => {
      scrollTopRef.current = el.scrollTop;
    };
    onScroll();
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, []);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (Math.abs(el.scrollTop - scrollTopRef.current) > 1) {
      el.scrollTop = scrollTopRef.current;
    }
  }, [displaySegments, tab.pendingDiffs.length]);

  useEffect(() => {
    if (activeHunkIndex == null || activeHunkIndex < 0) return;
    const range = lineRanges[activeHunkIndex];
    if (!range) return;
    if (hideBarTimerRef.current) {
      clearTimeout(hideBarTimerRef.current);
      hideBarTimerRef.current = null;
    }
    setHoveredBlockId(range.blockId);
    // Only scroll on intentional hunk navigation — not when lineRanges
    // rebuilds after accept/reject (that was jumping the viewport to top).
    const indexChanged = lastScrolledHunkRef.current !== activeHunkIndex;
    lastScrolledHunkRef.current = activeHunkIndex;
    if (!indexChanged) return;
    hunkRefs.current.get(range.blockId)?.scrollIntoView({
      block: "nearest",
      behavior: "smooth",
    });
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
        filePath={tab.path}
        paragraphBase={seg.tabStart}
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
    const showInsert = !!seg.after;
    // Line number comes from placement in the live document (correct even when
    // old_string/new_string are mid-line snippets).
    const lineLabel = paraLabels[seg.tabStart] ?? null;
    // Always edit the live line range — not the historical `after` snapshot.
    // Gating on after===live made the green block flip to readOnly as soon as
    // the user typed (text diverged from the original Edit payload).
    const liveInsert = sliceTabLines(tab.text, seg.tabStart, seg.tabEnd);
    const insertLabels = liveInsert.split("\n").map(() => lineLabel);
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
            lineLabels={[lineLabel]}
            value={line}
            readOnly
            ariaLabel="removed line"
          />
        ))}
        {showInsert && (
          <ReaderCodeMirror
            key={`ins-${seg.blockId}`}
            layout="segment"
            filePath={tab.path}
            paragraphBase={seg.tabStart}
            diffVariant="insert"
            diffSign="+"
            diffSignFirstLineOnly
            lineLabels={insertLabels}
            value={liveInsert}
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
