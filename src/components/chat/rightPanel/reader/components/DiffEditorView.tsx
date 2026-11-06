import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  buildLineParagraphLabels,
  normalizeReaderPath,
  type ReaderFileTab,
} from "../../../../../store/reader";
import { useReaderFind } from "../../../../../store/readerFind";
import { findInText, resolveFindScrollIndex, type TextRange } from "../../../../../utils/readerFind";
import {
  buildEditorDisplaySegments,
  buildPendingDiffLineRanges,
  isDiffTextEqual,
  type EditorDisplaySegment,
} from "../../../../../utils/inlineDiff";
import { DIFF_BAR_HIDE_MS } from "../constants";
import { estimateLineHeight, useLayoutMetrics } from "../hooks/useLayoutMetrics";
import { LazyMount } from "./LazyMount";
import { ReaderCodeMirror } from "../ReaderCodeMirror";
import { ReaderDiffActionBar } from "../ReaderDiffActionBar";
import {
  readerViewportTop,
  rememberReaderViewport,
} from "../../../../../utils/readerViewport";

/** Line/column in the LF document the diff segments edit (`\r` before `\n` is dropped). */
function lineColOf(text: string, index: number): { line: number; col: number } {
  let line = 0;
  let col = 0;
  const end = Math.max(0, Math.min(index, text.length));
  for (let i = 0; i < end; i += 1) {
    const ch = text.charCodeAt(i);
    if (ch === 10) {
      line += 1;
      col = 0;
    } else if (ch === 13 && text.charCodeAt(i + 1) === 10) {
      // CRLF: the carriage return is not a character in the segment editor.
    } else {
      col += 1;
    }
  }
  return { line, col };
}

function offsetInSegment(lines: string[], tabStart: number, line: number, col: number): number {
  let off = 0;
  for (let i = tabStart; i < line; i += 1) off += (lines[i]?.length ?? 0) + 1;
  return off + col;
}

function clipFindRange(
  lines: string[],
  tabStart: number,
  tabEnd: number,
  start: { line: number; col: number },
  end: { line: number; col: number },
): TextRange | null {
  if (end.line < tabStart || start.line > tabEnd) return null;
  const fromLine = Math.max(start.line, tabStart);
  const toLine = Math.min(end.line, tabEnd);
  const fromCol = start.line < tabStart ? 0 : start.col;
  const toCol = end.line > tabEnd ? (lines[tabEnd]?.length ?? 0) : end.col;
  const a = offsetInSegment(lines, tabStart, fromLine, fromCol);
  const b = offsetInSegment(lines, tabStart, toLine, toCol);
  if (b <= a) return null;
  return { start: a, end: b };
}

function textLinesCount(text: string): number {
  let n = 1;
  for (let i = text.indexOf("\n"); i >= 0; i = text.indexOf("\n", i + 1)) n += 1;
  return n;
}

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
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [scrollEl, setScrollEl] = useState<HTMLDivElement | null>(null);
  const setScrollNode = useCallback((node: HTMLDivElement | null) => {
    scrollRef.current = node;
    setScrollEl(node);
  }, []);
  const hunkRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const scrollTopRef = useRef(0);
  const lastScrolledHunkRef = useRef<number | null>(null);
  const [hoveredBlockId, setHoveredBlockId] = useState<string | null>(null);
  const hideBarTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const paraLabels = useMemo(() => buildLineParagraphLabels(tab.text), [tab.text]);
  // CodeMirror treats "\r\n" / lone "\r" as line breaks. Files with CRLF endings
  // would therefore hand each segment a phantom extra line (trailing "\r") and
  // make every sync / keystroke disagree with the document. Edit on LF-only
  // lines and restore the file's own line ending when writing back.
  const eol = useMemo(() => (tab.text.includes("\r\n") ? "\r\n" : "\n"), [tab.text]);
  const textLines = useMemo(
    () =>
      tab.text
        .split("\n")
        .map((l) => (l.endsWith("\r") ? l.slice(0, -1) : l)),
    [tab.text],
  );
  const metrics = useLayoutMetrics(scrollEl);
  const findOpen = useReaderFind((s) => s.open);
  const findQuery = useReaderFind((s) => s.query);
  const matchCase = useReaderFind((s) => s.matchCase);
  const matchIndex = useReaderFind((s) => s.matchIndex);
  const navEpoch = useReaderFind((s) => s.navEpoch);
  const findMatches = useReaderFind((s) => s.matches);

  const findOverlay = useMemo(() => {
    if (!findOpen || !findQuery.trim()) {
      return {
        ranges: [] as TextRange[],
        located: [] as Array<{
          range: TextRange;
          start: { line: number; col: number };
          end: { line: number; col: number };
        }>,
        activeIndex: -1,
        scrollToIndex: null as number | null,
      };
    }
    const ranges = findInText(tab.text, findQuery, matchCase);
    const activeMatch = matchIndex >= 0 ? (findMatches[matchIndex] ?? null) : null;
    const sameFile =
      !!activeMatch &&
      normalizeReaderPath(activeMatch.path) === normalizeReaderPath(tab.path);
    let activeIndex = -1;
    if (sameFile && activeMatch) {
      activeIndex = ranges.findIndex(
        (r) => r.start === activeMatch.start && r.end === activeMatch.end,
      );
      if (activeIndex < 0) {
        const fileMatches = findMatches.filter(
          (m) => normalizeReaderPath(m.path) === normalizeReaderPath(tab.path),
        );
        const ord = fileMatches.findIndex(
          (m) => m.start === activeMatch.start && m.end === activeMatch.end,
        );
        if (ord >= 0 && ord < ranges.length) activeIndex = ord;
      }
    }
    const scrollToIndex =
      sameFile && activeMatch
        ? resolveFindScrollIndex(
            tab.text,
            findQuery,
            matchCase,
            activeMatch,
            findMatches.filter(
              (m) => normalizeReaderPath(m.path) === normalizeReaderPath(tab.path),
            ),
          )
        : null;
    const located = ranges.map((range) => ({
      range,
      start: lineColOf(tab.text, range.start),
      end: lineColOf(tab.text, range.end),
    }));
    return { ranges, located, activeIndex, scrollToIndex };
  }, [findOpen, findQuery, matchCase, matchIndex, findMatches, tab.text, tab.path]);

  const findScrollRef = useRef<number | null>(null);
  findScrollRef.current = findOverlay.scrollToIndex;
  const findTextRef = useRef(tab.text);
  findTextRef.current = tab.text;

  const findForSpan = useCallback(
    (tabStart: number, tabEnd: number): { ranges: TextRange[]; activeIndex: number } => {
      const located = findOverlay.located;
      if (!located || located.length === 0) return { ranges: [], activeIndex: -1 };
      const ranges: TextRange[] = [];
      let activeIndex = -1;
      located.forEach((item, index) => {
        const clipped = clipFindRange(textLines, tabStart, tabEnd, item.start, item.end);
        if (!clipped) return;
        if (index === findOverlay.activeIndex) activeIndex = ranges.length;
        ranges.push(clipped);
      });
      return { ranges, activeIndex };
    },
    [findOverlay, textLines],
  );

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

  // Line ranges the user's own typing has pinned. Placement of a hunk is derived
  // from the agent's original `after` snippet; once the user edits inside the
  // green block that snippet no longer matches, the derived range drifts and the
  // block's editor is handed a different slice (full-document replace → the
  // caret is lost). Pin the range to what the user actually produced instead.
  const pinnedRangesRef = useRef<{
    text: string;
    ranges: Map<string, { start: number; end: number }>;
  } | null>(null);

  const lineRanges = useMemo(() => {
    const base = buildPendingDiffLineRanges(tab.text, diffBlocks);
    const pinned = pinnedRangesRef.current;
    // Only valid for the exact text our own edit produced; any outside change
    // (agent edit, reload) falls back to derived placement.
    if (!pinned || pinned.text !== tab.text) return base;
    const lastLine = Math.max(0, textLinesCount(tab.text) - 1);
    return base.map((r) => {
      const o = pinned.ranges.get(r.blockId);
      if (!o) return r;
      return {
        ...r,
        startLine: Math.min(o.start, lastLine),
        endLine: Math.min(Math.max(o.end, o.start), lastLine),
      };
    });
  }, [tab.text, diffBlocks]);

  const displaySegments = useMemo(
    () => buildEditorDisplaySegments(tab.text, diffBlocks, lineRanges),
    [tab.text, diffBlocks, lineRanges],
  );

  const rangeByBlockId = useMemo(() => {
    const map = new Map<string, { range: (typeof lineRanges)[number]; index: number }>();
    lineRanges.forEach((range, index) => map.set(range.blockId, { range, index }));
    return map;
  }, [lineRanges]);

  /** Every Edit id → the id of the (possibly merged) hunk that displays it. */
  const hunkKeyByBlockId = useMemo(() => {
    const map = new Map<string, string>();
    for (const seg of displaySegments) {
      if (seg.kind !== "hunk") continue;
      for (const id of seg.blockIds) map.set(id, seg.blockId);
    }
    return map;
  }, [displaySegments]);

  const hoveredHunkIndex = useMemo(
    () =>
      hoveredBlockId
        ? lineRanges.findIndex((r) => r.blockId === hoveredBlockId)
        : -1,
    [hoveredBlockId, lineRanges],
  );

  const onSegmentChange = useCallback(
    (tabStart: number, tabEnd: number, value: string, blockIds?: string[]) => {
      const nextLines = textLines.slice();
      nextLines.splice(tabStart, tabEnd - tabStart + 1, ...value.split("\n"));
      const nextText = nextLines.join(eol);
      const newCount = value.split("\n").length;
      const delta = newCount - (tabEnd - tabStart + 1);

      const prev = pinnedRangesRef.current;
      const prevRanges = prev && prev.text === tab.text ? prev.ranges : null;
      const nextRanges = new Map<string, { start: number; end: number }>();
      if (prevRanges) {
        for (const [id, r] of prevRanges) {
          if (blockIds?.includes(id)) continue;
          // Pinned hunks below the edit move with the line-count change.
          nextRanges.set(
            id,
            r.start > tabEnd ? { start: r.start + delta, end: r.end + delta } : r,
          );
        }
      }
      if (blockIds) {
        // Every Edit merged into this hunk shares the one live line span.
        const span = { start: tabStart, end: tabStart + newCount - 1 };
        for (const id of blockIds) nextRanges.set(id, span);
      }
      // Must be in place before the store update re-renders with `nextText`.
      pinnedRangesRef.current = { text: nextText, ranges: nextRanges };
      applyText(nextText);
    },
    [applyText, tab.text, textLines, eol],
  );

  const showBarForBlock = useCallback(
    (blockId: string | null) => {
      if (hideBarTimerRef.current) {
        clearTimeout(hideBarTimerRef.current);
        hideBarTimerRef.current = null;
      }
      setHoveredBlockId(blockId);
      if (blockId && onActiveHunkChange) {
        const idx = rangeByBlockId.get(blockId)?.index ?? -1;
        if (idx >= 0) onActiveHunkChange(idx);
      }
    },
    [rangeByBlockId, onActiveHunkChange],
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
      // Edits merged into one hunk are a single stop: skip past the rest of them.
      const groupOf = (i: number) => {
        const id = lineRanges[i]?.blockId;
        return id != null ? (hunkKeyByBlockId.get(id) ?? id) : null;
      };
      const startGroup = groupOf(base);
      let nextIdx = base + direction;
      while (
        lineRanges[nextIdx] &&
        startGroup != null &&
        groupOf(nextIdx) === startGroup
      ) {
        nextIdx += direction;
      }
      const next = lineRanges[nextIdx];
      if (!next) return;
      showBarForBlock(next.blockId);
      onActiveHunkChange?.(nextIdx);
      hunkRefs.current.get(next.blockId)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    },
    [
      hoveredHunkIndex,
      activeHunkIndex,
      lineRanges,
      hunkKeyByBlockId,
      showBarForBlock,
      onActiveHunkChange,
    ],
  );

  useEffect(() => {
    lastScrolledHunkRef.current = null;
  }, [tab.path]);

  // Restore the last source viewport, then track scroll. Do not write 0 on
  // mount — that would clobber the position saved by the plain editor.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const saved = readerViewportTop(tab.path);
    el.scrollTop = saved;
    scrollTopRef.current = el.scrollTop;
    const onScroll = () => {
      scrollTopRef.current = el.scrollTop;
      rememberReaderViewport(tab.path, el.scrollTop);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      rememberReaderViewport(tab.path, el.scrollTop);
      el.removeEventListener("scroll", onScroll);
    };
  }, [tab.path]);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (Math.abs(el.scrollTop - scrollTopRef.current) > 1) {
      el.scrollTop = scrollTopRef.current;
    }
  }, [displaySegments, tab.pendingDiffs.length]);

  // Keep / Undo restructure the segments (a hunk collapses into plain context
  // and neighbouring context blocks merge, remounting editors with estimated
  // heights). Pin the document line that was at the top of the viewport to the
  // same screen position until the layout settles, instead of letting the
  // scroll jump to the top or the middle.
  const anchorRef = useRef<{ line: number; offset: number; at: number } | null>(null);
  const anchorRafRef = useRef<number | null>(null);

  const cancelAnchor = useCallback(() => {
    anchorRef.current = null;
    if (anchorRafRef.current != null) {
      cancelAnimationFrame(anchorRafRef.current);
      anchorRafRef.current = null;
    }
  }, []);

  /** Rendered rows of a segment, one per live document line (green side for hunks). */
  const liveRows = (child: HTMLElement): ArrayLike<Element> => {
    const host =
      child.dataset.segKind === "hunk"
        ? (child.querySelector(".reader-codemirror--insert") ?? child)
        : child;
    return host.querySelectorAll(".cm-line");
  };

  const captureAnchor = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    const top = el.getBoundingClientRect().top;
    for (const node of Array.from(el.children)) {
      const child = node as HTMLElement;
      const rect = child.getBoundingClientRect();
      if (rect.bottom <= top + 1) continue;
      const start = Number(child.dataset.segStart);
      if (!Number.isFinite(start)) return;
      let line = start;
      let lineTop = rect.top;
      // Anchor on the exact visible document line, never on a far-away block
      // start: pixel distances across estimated-height placeholders are wrong.
      const rows = liveRows(child);
      for (let k = 0; k < rows.length; k += 1) {
        const r = rows[k].getBoundingClientRect();
        if (r.bottom > top + 1) {
          line = start + k;
          lineTop = r.top;
          break;
        }
      }
      anchorRef.current = { line, offset: lineTop - top, at: performance.now() };
      return;
    }
  }, []);

  useLayoutEffect(() => {
    const el = scrollRef.current;
    const anchor = anchorRef.current;
    if (!el || !anchor || performance.now() - anchor.at > 2000) return;

    const locate = (): number | null => {
      for (const node of Array.from(el.children)) {
        const child = node as HTMLElement;
        const s = Number(child.dataset.segStart);
        const e = Number(child.dataset.segEnd);
        if (!(anchor.line >= s && anchor.line <= e)) continue;
        const target = liveRows(child)[anchor.line - s];
        // Not mounted yet (placeholder): wait for the real editor instead of
        // guessing from an estimated height.
        if (!target) return null;
        return target.getBoundingClientRect().top;
      }
      return null;
    };
    const correct = () => {
      const now = locate();
      if (now == null) return;
      const delta = now - el.getBoundingClientRect().top - anchor.offset;
      if (Math.abs(delta) > 0.5) el.scrollTop += delta;
    };

    correct();
    // Placeholders keep swapping for real editors for a moment; keep correcting.
    const until = performance.now() + 700;
    const tick = () => {
      if (anchorRef.current !== anchor) return;
      correct();
      if (performance.now() < until) {
        anchorRafRef.current = requestAnimationFrame(tick);
      } else {
        anchorRafRef.current = null;
        anchorRef.current = null;
      }
    };
    if (anchorRafRef.current != null) cancelAnimationFrame(anchorRafRef.current);
    anchorRafRef.current = requestAnimationFrame(tick);
  }, [displaySegments]);

  useEffect(() => cancelAnchor, [cancelAnchor]);

  useEffect(() => {
    if (activeHunkIndex == null) return;
    if (activeHunkIndex < 0) {
      // Stay off-hunk without consuming the first navigation as a no-scroll paint.
      lastScrolledHunkRef.current = -1;
      return;
    }
    const range = lineRanges[activeHunkIndex];
    if (!range) return;
    if (hideBarTimerRef.current) {
      clearTimeout(hideBarTimerRef.current);
      hideBarTimerRef.current = null;
    }
    setHoveredBlockId(range.blockId);
    // Only scroll on intentional hunk navigation (the header arrows / bar).
    // First paint and incoming agent edits must leave the viewport alone —
    // the user may be reading a different part of this file.
    if (lastScrolledHunkRef.current === null) {
      lastScrolledHunkRef.current = activeHunkIndex;
      return;
    }
    const indexChanged = lastScrolledHunkRef.current !== activeHunkIndex;
    lastScrolledHunkRef.current = activeHunkIndex;
    if (!indexChanged) return;
    hunkRefs.current.get(range.blockId)?.scrollIntoView({
      block: "nearest",
      behavior: "smooth",
    });
  }, [activeHunkIndex, lineRanges]);

  useEffect(() => {
    const index = findScrollRef.current;
    const root = scrollRef.current;
    if (index == null || !root) return;
    const { line } = lineColOf(findTextRef.current, index);
    const nodes = root.querySelectorAll<HTMLElement>("[data-seg-start]");
    let host: HTMLElement | null = null;
    for (const node of nodes) {
      const start = Number(node.dataset.segStart);
      const end = Number(node.dataset.segEnd);
      if (line >= start && line <= end) {
        host = node;
        break;
      }
    }
    if (!host) return;
    const localLine = line - Number(host.dataset.segStart);
    const scrollLine = () => {
      const row = host!.querySelectorAll(
        ".reader-codemirror:not(.reader-codemirror--delete) .cm-line",
      )[localLine];
      if (!row) return false;
      row.scrollIntoView({ block: "center", behavior: "smooth" });
      return true;
    };
    if (scrollLine()) return;
    host.scrollIntoView({ block: "nearest" });
    requestAnimationFrame(() => {
      if (!scrollLine()) requestAnimationFrame(() => scrollLine());
    });
  }, [navEpoch]);

  const estimateLines = (from: number, to: number) => {
    let total = 0;
    for (let i = from; i <= to; i += 1) {
      total += estimateLineHeight(textLines[i]?.length ?? 0, metrics);
    }
    return total;
  };

  const renderContextBlock = (key: string, tabStart: number, tabEnd: number) => (
    <div key={key} data-seg-kind="context" data-seg-start={tabStart} data-seg-end={tabEnd}>
    <LazyMount
      root={scrollEl}
      estimateHeight={() => estimateLines(tabStart, tabEnd)}
    >
      {() => {
        const segmentLabels: (number | null)[] = [];
        for (let i = tabStart; i <= tabEnd; i += 1) {
          segmentLabels.push(paraLabels[i] ?? null);
        }
        const find = findForSpan(tabStart, tabEnd);
        return (
          <ReaderCodeMirror
            layout="segment"
            filePath={tab.path}
            paragraphBase={tabStart}
            lineLabels={segmentLabels}
            value={textLines.slice(tabStart, tabEnd + 1).join("\n")}
            onChange={(value) => onSegmentChange(tabStart, tabEnd, value)}
            ariaLabel={tab.path}
            findRanges={find.ranges}
            findActiveIndex={find.activeIndex}
          />
        );
      }}
    </LazyMount>
    </div>
  );

  const renderHunk = (seg: Extract<EditorDisplaySegment, { kind: "hunk" }>) => {
    const merged = seg.blockIds.length > 1;
    // A merged hunk's red side is the original span, so compare it with the live
    // text; a single Edit compares its own before/after snippets.
    const unchanged = merged
      ? isDiffTextEqual(seg.before, textLines.slice(seg.tabStart, seg.tabEnd + 1).join("\n"))
      : isDiffTextEqual(seg.before, seg.after);
    if (unchanged) {
      return renderContextBlock(`ctx-hunk-${seg.blockId}`, seg.tabStart, seg.tabEnd);
    }

    const showDelete = !!seg.before.trim();
    const showInsert = !!seg.after;
    // Number every line from where this hunk sits in the current document.
    // Reusing only the first line's label made a multi-line block read 1, 1, 1…
    const firstLineNo = paraLabels[seg.tabStart] ?? seg.tabStart + 1;
    const located = rangeByBlockId.get(seg.blockId);
    const range = located?.range;
    const hovered =
      hoveredBlockId != null && seg.blockIds.includes(hoveredBlockId)
        ? rangeByBlockId.get(hoveredBlockId)
        : undefined;
    const hunkIndex = (hovered ?? located)?.index ?? -1;
    const showBar = hovered != null && range != null && hunkIndex >= 0;

    const estimateHunk = () => {
      let h = 0;
      if (showDelete) {
        for (const line of seg.before.split("\n")) {
          h += estimateLineHeight(line.length, metrics);
        }
      }
      if (showInsert) h += estimateLines(seg.tabStart, seg.tabEnd);
      return h;
    };

    return (
      <div
        key={seg.blockId}
        ref={(el) => {
          for (const id of seg.blockIds) {
            if (el) hunkRefs.current.set(id, el);
            else hunkRefs.current.delete(id);
          }
        }}
        className="reader-editor-hunk"
        data-seg-kind="hunk"
        data-seg-start={seg.tabStart}
        data-seg-end={seg.tabEnd}
        onMouseEnter={() => showBarForBlock(seg.blockId)}
        onMouseLeave={scheduleHideBar}
      >
        <LazyMount root={scrollEl} estimateHeight={estimateHunk}>
          {() => {
            // Always edit the live line range — not the historical `after`
            // snapshot. Gating on after===live made the green block flip to
            // readOnly as soon as the user typed (text diverged from the
            // original Edit payload).
            const liveInsert = textLines.slice(seg.tabStart, seg.tabEnd + 1).join("\n");
            const find = findForSpan(seg.tabStart, seg.tabEnd);
            const insertLabels = liveInsert.split("\n").map((_, i) =>
              paraLabels[seg.tabStart + i] ?? firstLineNo + i,
            );
            // Red lines are the original span, which can be longer or shorter
            // than the live text, so count forward from the same start.
            const deleteLabels = seg.before.split("\n").map((_, i) => firstLineNo + i);
            return (
              <>
                {showDelete && (
                  // One read-only editor for the whole removed block, not one per line.
                  <ReaderCodeMirror
                    layout="segment"
                    diffVariant="delete"
                    diffSign="−"
                    lineLabels={deleteLabels}
                    value={seg.before}
                    readOnly
                    ariaLabel="removed line"
                  />
                )}
                {showInsert && (
                  <ReaderCodeMirror
                    layout="segment"
                    filePath={tab.path}
                    paragraphBase={seg.tabStart}
                    diffVariant="insert"
                    diffSign="+"
                    diffSignFirstLineOnly
                    lineLabels={insertLabels}
                    value={liveInsert}
                    onChange={(value) =>
                      onSegmentChange(seg.tabStart, seg.tabEnd, value, seg.blockIds)
                    }
                    ariaLabel={tab.path}
                    findRanges={find.ranges}
                    findActiveIndex={find.activeIndex}
                  />
                )}
              </>
            );
          }}
        </LazyMount>
        {showBar && (
          <ReaderDiffActionBar
            tab={tab}
            range={range}
            blockIds={seg.blockIds}
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

  // Context blocks are keyed by the hunk that precedes them so their editors
  // survive line-count changes (line-index keys would remount everything below
  // the edit on every Enter / Backspace).
  const renderedSegments: ReactNode[] = [];
  let prevHunkId = "start";
  for (const seg of displaySegments) {
    if (seg.kind === "context") {
      renderedSegments.push(
        renderContextBlock(`ctx-after-${prevHunkId}`, seg.tabStart, seg.tabEnd),
      );
    } else {
      renderedSegments.push(renderHunk(seg));
      prevHunkId = seg.blockId;
    }
  }

  return (
    <div className="reader-editor-wrap reader-editor-wrap--diff reader-editor-wrap--codemirror">
      <div className="reader-editor-main" onMouseLeave={scheduleHideBar}>
        <div
          ref={setScrollNode}
          className="reader-editor-scroll"
          onMouseDownCapture={captureAnchor}
          onWheel={cancelAnchor}
        >
          {renderedSegments}
        </div>
      </div>
    </div>
  );
}
