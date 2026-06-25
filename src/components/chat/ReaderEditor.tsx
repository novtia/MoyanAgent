import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useSession } from "../../store/session";
import {
  buildLineParagraphLabels,
  normalizeReaderPath,
  useReader,
  type ReaderFileTab,
} from "../../store/reader";
import { useReaderFind } from "../../store/readerFind";
import { findInText, resolveFindScrollIndex } from "../../utils/readerFind";
import { ReaderCodeMirror } from "./ReaderCodeMirror";
import { api } from "../../api/tauri";
import {
  buildEditorDisplaySegments,
  buildPendingDiffLineRanges,
  replaceTabLineRange,
  sliceTabLines,
  type EditorDisplaySegment,
} from "../../utils/inlineDiff";
import { ReaderDiffActionBar } from "./ReaderDiffActionBar";

const SAVE_DEBOUNCE_MS = 600;
/** Delay before hiding diff action bar after pointer leaves the hunk. */
const DIFF_BAR_HIDE_MS = 480;

interface ReaderEditorProps {
  tab: ReaderFileTab;
  activeHunkIndex?: number;
  onActiveHunkChange?: (index: number) => void;
}

function PlainReader({
  tab,
  applyText,
}: {
  tab: ReaderFileTab;
  applyText: (text: string) => void;
}) {
  const findOpen = useReaderFind((s) => s.open);
  const findQuery = useReaderFind((s) => s.query);
  const matchCase = useReaderFind((s) => s.matchCase);
  const matchIndex = useReaderFind((s) => s.matchIndex);
  const findMatches = useReaderFind((s) => s.matches);

  const { ranges: findRanges, activeIndex: findActiveIndex } = useMemo(() => {
    if (!findOpen || !findQuery.trim() || tab.pendingDiffs.length > 0) {
      return { ranges: [], activeIndex: -1 };
    }
    const ranges = findInText(tab.text, findQuery, matchCase);
    if (matchIndex < 0) {
      return { ranges, activeIndex: -1 };
    }
    const activeMatch = findMatches[matchIndex] ?? null;
    if (
      !activeMatch ||
      normalizeReaderPath(activeMatch.path) !== normalizeReaderPath(tab.path)
    ) {
      return { ranges, activeIndex: -1 };
    }
    const activeIndex = ranges.findIndex(
      (r) => r.start === activeMatch.start && r.end === activeMatch.end,
    );
    if (activeIndex >= 0) {
      return { ranges, activeIndex };
    }
    const fileMatches = findMatches.filter(
      (m) => normalizeReaderPath(m.path) === normalizeReaderPath(tab.path),
    );
    const ord = fileMatches.findIndex(
      (m) => m.start === activeMatch.start && m.end === activeMatch.end,
    );
    return { ranges, activeIndex: ord >= 0 ? ord : -1 };
  }, [
    findOpen,
    findQuery,
    matchCase,
    tab.text,
    tab.path,
    tab.pendingDiffs.length,
    findMatches,
    matchIndex,
  ]);

  const showFindHighlight =
    findOpen && findQuery.trim().length > 0 && findRanges.length > 0;

  const scrollToIndex = useMemo(() => {
    if (!findOpen || matchIndex < 0 || tab.pendingDiffs.length > 0) return null;
    const activeMatch = findMatches[matchIndex] ?? null;
    if (
      !activeMatch ||
      normalizeReaderPath(activeMatch.path) !== normalizeReaderPath(tab.path)
    ) {
      return null;
    }
    const fileMatches = findMatches.filter(
      (m) => normalizeReaderPath(m.path) === normalizeReaderPath(tab.path),
    );
    return resolveFindScrollIndex(
      tab.text,
      findQuery,
      matchCase,
      activeMatch,
      fileMatches,
    );
  }, [
    findOpen,
    matchIndex,
    tab.path,
    tab.text,
    tab.pendingDiffs.length,
    findQuery,
    matchCase,
    findMatches,
  ]);

  return (
    <div className="reader-editor-wrap reader-editor-wrap--plain reader-editor-wrap--codemirror">
      <ReaderCodeMirror
        value={tab.text}
        onChange={applyText}
        ariaLabel={tab.path}
        findRanges={showFindHighlight ? findRanges : []}
        findActiveIndex={showFindHighlight ? findActiveIndex : -1}
        scrollToIndex={scrollToIndex}
        scrollTrigger={matchIndex}
      />
    </div>
  );
}

export function ReaderEditor({ tab, activeHunkIndex, onActiveHunkChange }: ReaderEditorProps) {
  const sessionId = useSession((s) => s.activeId);
  const updateTabText = useReader((s) => s.updateTabText);
  const setTabDirty = useReader((s) => s.setTabDirty);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latestTextRef = useRef(tab.text);
  const dirtyRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  const hunkRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const [hoveredBlockId, setHoveredBlockId] = useState<string | null>(null);
  const hideBarTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const hasPendingDiff = tab.pendingDiffs.length > 0;
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
    () => (hasPendingDiff ? buildPendingDiffLineRanges(tab.text, diffBlocks) : []),
    [tab.text, diffBlocks, hasPendingDiff],
  );

  const displaySegments = useMemo(
    () => (hasPendingDiff ? buildEditorDisplaySegments(tab.text, diffBlocks) : []),
    [tab.text, diffBlocks, hasPendingDiff],
  );

  const hoveredHunkIndex = useMemo(
    () =>
      hoveredBlockId
        ? lineRanges.findIndex((r) => r.blockId === hoveredBlockId)
        : -1,
    [hoveredBlockId, lineRanges],
  );

  useEffect(() => {
    latestTextRef.current = tab.text;
    if (!tab.dirty) dirtyRef.current = false;
  }, [tab.text, tab.dirty]);

  const flushSave = useCallback(
    async (text: string) => {
      if (!sessionId || !tab.path) return;
      try {
        await api.writeProjectFile(sessionId, tab.path, text, tab.encoding, tab.hadBom);
        dirtyRef.current = false;
        setTabDirty(tab.path, false, false);
      } catch {
        setTabDirty(tab.path, true, true);
      }
    },
    [sessionId, tab.path, tab.encoding, tab.hadBom, setTabDirty],
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
        void api
          .writeProjectFile(
            sessionId,
            tab.path,
            latestTextRef.current,
            tab.encoding,
            tab.hadBom,
          )
          .catch(() => {
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
    if (!hasPendingDiff || activeHunkIndex == null || activeHunkIndex < 0) return;
    const range = lineRanges[activeHunkIndex];
    if (!range) return;
    if (hideBarTimerRef.current) {
      clearTimeout(hideBarTimerRef.current);
      hideBarTimerRef.current = null;
    }
    setHoveredBlockId(range.blockId);
    hunkRefs.current.get(range.blockId)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [activeHunkIndex, hasPendingDiff, lineRanges]);

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

  if (!hasPendingDiff) {
    return <PlainReader tab={tab} applyText={applyText} />;
  }

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
