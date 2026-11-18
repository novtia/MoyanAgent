import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../../../api/tauri";
import {
  normalizeReaderPath,
  type ReaderFileTab,
} from "../../../../../store/reader";
import { useReaderFind } from "../../../../../store/readerFind";
import type { RewriteMode } from "../../../../../store/readerRewrite";
import { useSettings } from "../../../../../store/settings";
import { findInText, resolveFindScrollIndex, type TextRange } from "../../../../../utils/readerFind";
import type {
  InlineWordDiffResult,
  PendingDiffLineRange,
} from "../../../../../utils/inlineDiff";
import {
  buildChunksFromRanges,
  splitDocumentLines,
  splitLinesIntoParagraphChunks,
  type ReaderParagraphChunk,
} from "../../../../../utils/readerChunks";
import {
  flushReaderProgress,
  hasReaderViewport,
  readerSavedLine,
  readerViewportTop,
  rememberReaderLine,
  rememberReaderViewport,
} from "../../../../../utils/readerViewport";
import { DEFAULT_REWRITE_SETTINGS } from "../../../../settings/rewrite/RewriteSection";
import { DIFF_BAR_HIDE_MS } from "../constants";
import { estimateLineHeight, useLayoutMetrics } from "../hooks/useLayoutMetrics";
import { ReaderCodeMirror } from "../ReaderCodeMirror";
import { ReaderDiffActionBar } from "../ReaderDiffActionBar";
import { LazyMount } from "./LazyMount";
import { markReaderOpenBackend } from "../../../../../utils/readerPerf";

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
      // CRLF
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

export interface ChunkRewriteTarget {
  text: string;
  startLine: number;
  endLine: number;
}

interface ChunkGroupDiffInfo {
  inlineDiff: InlineWordDiffResult;
  pureDeletes: Array<{ blockId: string; before: string; tabStart: number }>;
  range: PendingDiffLineRange | null;
  hunkIndex: number;
  blockIds: string[];
}

interface ChunkGroupItemProps {
  chunk: ReturnType<typeof buildChunksFromRanges>[number];
  tab: ReaderFileTab;
  scrollEl: HTMLDivElement | null;
  estimatedHeight: number;
  findRanges: TextRange[];
  findActiveIndex: number;
  diffInfo: ChunkGroupDiffInfo | null;
  showBar: boolean;
  hunkTotal: number;
  rewriteRunning: boolean;
  hasRewriteHandler: boolean;
  onRewriteChunk?: (mode: RewriteMode, target: ChunkRewriteTarget) => void;
  onChunkChange: (chunkIndex: number, startLine: number, endLine: number, value: string) => void;
  onFocusChange: (chunkIndex: number, focused: boolean) => void;
  onShowBar: (chunkIdx: number, hunkIdx: number) => void;
  onScheduleHideBar: () => void;
  onNavigateHunk: (direction: -1 | 1) => void;
  registerHunkRef: (blockIds: string[], el: HTMLElement | null) => void;
}

function sameFindRanges(a: TextRange[], b: TextRange[]): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (a[i].start !== b[i].start || a[i].end !== b[i].end) return false;
  }
  return true;
}

function sameInlineDiff(
  a: InlineWordDiffResult | null | undefined,
  b: InlineWordDiffResult | null | undefined,
): boolean {
  if (a === b) return true;
  if (!a || !b) return !a && !b;
  if (a.added.length !== b.added.length || a.removed.length !== b.removed.length) return false;
  for (let i = 0; i < a.added.length; i += 1) {
    if (a.added[i].from !== b.added[i].from || a.added[i].to !== b.added[i].to) return false;
  }
  for (let i = 0; i < a.removed.length; i += 1) {
    if (a.removed[i].pos !== b.removed[i].pos || a.removed[i].text !== b.removed[i].text) {
      return false;
    }
  }
  return true;
}

const ChunkGroupItem = memo(
  function ChunkGroupItem({
    chunk,
    tab,
    scrollEl,
    estimatedHeight,
    findRanges,
    findActiveIndex,
    diffInfo,
    showBar,
    hunkTotal,
    rewriteRunning,
    hasRewriteHandler,
    onRewriteChunk,
    onChunkChange,
    onFocusChange,
    onShowBar,
    onScheduleHideBar,
    onNavigateHunk,
    registerHunkRef,
  }: ChunkGroupItemProps) {
    const { t } = useTranslation();
    const pFrom = chunk.startLine + 1;
    const pTo = chunk.endLine + 1;
    const rangeText = pFrom === pTo ? `P${pFrom}` : `P${pFrom}–P${pTo}`;
    const blockIds = diffInfo?.blockIds;

    return (
      <section
        ref={(el) => {
          if (blockIds) registerHunkRef(blockIds, el);
        }}
        className={`reader-chunk-group${diffInfo ? " has-diff" : ""}`}
        data-seg-kind="chunk"
        data-seg-start={chunk.startLine}
        data-seg-end={chunk.endLine}
        onMouseEnter={() => {
          if (diffInfo) onShowBar(chunk.index, diffInfo.hunkIndex);
        }}
        onMouseLeave={() => {
          if (diffInfo) onScheduleHideBar();
        }}
      >
        <div className="reader-chunk-header">
          <span className="reader-chunk-meta">
            {t("reader.chunkGroupTitle", {
              index: chunk.index + 1,
              range: rangeText,
              chars: chunk.chars,
            })}
          </span>
          {hasRewriteHandler && onRewriteChunk && (
            <div className="reader-chunk-actions">
              <button
                type="button"
                className="reader-chunk-btn"
                disabled={rewriteRunning || !chunk.text.trim()}
                onClick={() =>
                  onRewriteChunk("rewrite", {
                    text: chunk.text,
                    startLine: chunk.startLine,
                    endLine: chunk.endLine,
                  })
                }
              >
                {t("reader.chunkRewrite")}
              </button>
              <button
                type="button"
                className="reader-chunk-btn"
                disabled={rewriteRunning || !chunk.text.trim()}
                onClick={() =>
                  onRewriteChunk("expand", {
                    text: chunk.text,
                    startLine: chunk.startLine,
                    endLine: chunk.endLine,
                  })
                }
              >
                {t("reader.chunkExpand")}
              </button>
            </div>
          )}
        </div>
        <LazyMount root={scrollEl} estimateHeight={() => estimatedHeight}>
          {() => (
            <>
              {diffInfo?.pureDeletes.map((del) => (
                <ReaderCodeMirror
                  key={`del-${del.blockId}`}
                  layout="segment"
                  diffVariant="delete"
                  diffSign="−"
                  lineLabels={del.before.split("\n").map((_, i) => del.tabStart + 1 + i)}
                  value={del.before}
                  readOnly
                  ariaLabel="removed line"
                />
              ))}
              <ReaderCodeMirror
                layout="segment"
                filePath={tab.path}
                paragraphBase={chunk.startLine}
                lineLabels={chunk.labels}
                value={chunk.text}
                inlineDiff={diffInfo?.inlineDiff ?? null}
                onChange={(value) =>
                  onChunkChange(chunk.index, chunk.startLine, chunk.endLine, value)
                }
                onFocusChange={(focused) => onFocusChange(chunk.index, focused)}
                ariaLabel={`${tab.path} (${rangeText})`}
                findRanges={findRanges}
                findActiveIndex={findActiveIndex}
              />
            </>
          )}
        </LazyMount>
        {showBar && diffInfo && diffInfo.range && (
          <ReaderDiffActionBar
            tab={tab}
            range={diffInfo.range}
            blockIds={diffInfo.blockIds}
            hunkIndex={diffInfo.hunkIndex}
            hunkTotal={hunkTotal}
            onNavigate={onNavigateHunk}
            onMouseEnter={() => onShowBar(chunk.index, diffInfo.hunkIndex)}
            onMouseLeave={onScheduleHideBar}
          />
        )}
      </section>
    );
  },
  (prev, next) =>
    prev.chunk.index === next.chunk.index &&
    prev.chunk.startLine === next.chunk.startLine &&
    prev.chunk.endLine === next.chunk.endLine &&
    prev.chunk.chars === next.chunk.chars &&
    prev.chunk.text === next.chunk.text &&
    prev.tab.path === next.tab.path &&
    prev.scrollEl === next.scrollEl &&
    prev.findActiveIndex === next.findActiveIndex &&
    sameFindRanges(prev.findRanges, next.findRanges) &&
    prev.showBar === next.showBar &&
    prev.hunkTotal === next.hunkTotal &&
    prev.rewriteRunning === next.rewriteRunning &&
    prev.hasRewriteHandler === next.hasRewriteHandler &&
    prev.onRewriteChunk === next.onRewriteChunk &&
    prev.diffInfo?.hunkIndex === next.diffInfo?.hunkIndex &&
    (prev.diffInfo?.pureDeletes.length ?? 0) === (next.diffInfo?.pureDeletes.length ?? 0) &&
    sameInlineDiff(prev.diffInfo?.inlineDiff, next.diffInfo?.inlineDiff) &&
    (!next.showBar || prev.tab.pendingDiffs === next.tab.pendingDiffs),
);

export function ChunkedEditorView({
  tab,
  applyText,
  rewriteRunning,
  onRewriteChunk,
  activeHunkIndex,
  onActiveHunkChange,
}: {
  tab: ReaderFileTab;
  applyText: (text: string) => void;
  rewriteRunning?: boolean;
  onRewriteChunk?: (mode: RewriteMode, target: ChunkRewriteTarget) => void;
  activeHunkIndex?: number;
  onActiveHunkChange?: (index: number) => void;
}) {
  const { t } = useTranslation();
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const [scrollEl, setScrollEl] = useState<HTMLDivElement | null>(null);
  const setScrollNode = useCallback((node: HTMLDivElement | null) => {
    scrollRef.current = node;
    setScrollEl(node);
  }, []);
  const scrollTopRef = useRef(0);
  const hunkRefs = useRef<Map<string, HTMLElement>>(new Map());
  const lastScrolledHunkRef = useRef<number | null>(null);
  const [hoveredChunkIdx, setHoveredChunkIdx] = useState<number | null>(null);
  const hideBarTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const metrics = useLayoutMetrics(scrollEl);

  const chunkChars = useSettings(
    (s) => s.settings?.rewrite?.chunk_chars ?? DEFAULT_REWRITE_SETTINGS.chunk_chars,
  );

  const { lines: textLines, eol } = useMemo(() => splitDocumentLines(tab.text), [tab.text]);

  // Keep chunk ranges stable across local edits and per-chunk rewrites:
  // only re-split the whole document when switching files or changing chunkChars.
  // Heavy calculations (diff placement, word-level Myers diff, paragraph word counts)
  // are offloaded to Rust via `api.computeReaderChunkLayout` so the UI thread stays responsive.
  const [focusedChunkIdx, setFocusedChunkIdx] = useState<number | null>(null);
  const pinnedRangesRef = useRef<{
    chunkChars: number;
    path: string;
    lineCount: number;
    ranges: Array<{ startLine: number; endLine: number; chars: number }>;
  } | null>(null);

  const [rustLayout, setRustLayout] = useState<{
    path: string;
    text: string;
    pendingDiffs: ReaderFileTab["pendingDiffs"];
    chunkChars: number;
    chunks: ReaderParagraphChunk[];
    lineRanges: PendingDiffLineRange[];
    diffByChunk: Map<number, ChunkGroupDiffInfo>;
  } | null>(null);

  useEffect(() => {
    pinnedRangesRef.current = null;
    setFocusedChunkIdx(null);
  }, [tab.path, chunkChars]);

  const layoutReqIdRef = useRef(0);
  useEffect(() => {
    const reqId = ++layoutReqIdRef.current;
    const pinned = pinnedRangesRef.current;
    const hasPinned =
      pinned != null && pinned.path === tab.path && pinned.chunkChars === chunkChars;
    const tLayoutStart = performance.now();

    void api
      .computeReaderChunkLayout({
        text: tab.text,
        chunkChars,
        blocks: tab.pendingDiffs.map((d) => ({
          id: d.id,
          before: d.before,
          after: d.after,
          textBefore: d.textBefore,
          textAfter: d.textAfter,
          paragraphNumber: d.paragraphNumber,
        })),
        pinnedRanges: hasPinned
          ? pinned.ranges.map((r) => ({ startLine: r.startLine, endLine: r.endLine }))
          : null,
        prevLineCount: hasPinned ? pinned.lineCount : null,
        focusedChunkIdx,
      })
      .then((res) => {
        if (layoutReqIdRef.current !== reqId) return;
        markReaderOpenBackend(
          tab.path,
          "backendChunkLayoutMs",
          performance.now() - tLayoutStart,
          { chunksCount: res.chunks.length },
        );
        const nextRanges = res.chunks.map((c) => ({
          startLine: c.startLine,
          endLine: c.endLine,
          chars: c.chars,
        }));
        pinnedRangesRef.current = {
          chunkChars,
          path: tab.path,
          lineCount: res.lineCount,
          ranges: nextRanges,
        };
        const chunks = buildChunksFromRanges(textLines, nextRanges);
        const diffByChunk = new Map<number, ChunkGroupDiffInfo>();
        for (const cd of res.chunkDiffs) {
          diffByChunk.set(cd.chunkIndex, {
            inlineDiff: cd.inlineDiff,
            pureDeletes: cd.pureDeletes,
            range: cd.range,
            hunkIndex: cd.hunkIndex,
            blockIds: cd.blockIds,
          });
        }
        setRustLayout({
          path: tab.path,
          text: tab.text,
          pendingDiffs: tab.pendingDiffs,
          chunkChars,
          chunks,
          lineRanges: res.lineRanges,
          diffByChunk,
        });
      })
      .catch(() => {
        // Fallback handled synchronously below if IPC ever fails
      });
  }, [tab.path, tab.text, tab.pendingDiffs, chunkChars, focusedChunkIdx, textLines]);

  const chunks = useMemo(() => {
    if (
      rustLayout &&
      rustLayout.path === tab.path &&
      rustLayout.text === tab.text &&
      rustLayout.chunkChars === chunkChars
    ) {
      return rustLayout.chunks;
    }
    const pinned = pinnedRangesRef.current;
    if (pinned && pinned.path === tab.path && pinned.chunkChars === chunkChars) {
      if (pinned.lineCount === textLines.length) {
        return buildChunksFromRanges(textLines, pinned.ranges);
      }
      const delta = textLines.length - pinned.lineCount;
      const targetIdx = focusedChunkIdx ?? 0;
      if (targetIdx >= 0 && targetIdx < pinned.ranges.length) {
        const adjusted = pinned.ranges.map((r, i) => {
          if (i < targetIdx) return r;
          if (i === targetIdx) {
            return {
              startLine: r.startLine,
              endLine: Math.max(r.startLine, r.endLine + delta),
              chars: r.chars,
            };
          }
          return {
            startLine: Math.max(0, r.startLine + delta),
            endLine: Math.max(0, r.endLine + delta),
            chars: r.chars,
          };
        });
        return buildChunksFromRanges(textLines, adjusted);
      }
    }
    const fresh = splitLinesIntoParagraphChunks(textLines, chunkChars);
    pinnedRangesRef.current = {
      chunkChars,
      path: tab.path,
      lineCount: textLines.length,
      ranges: fresh.map((c) => ({
        startLine: c.startLine,
        endLine: c.endLine,
        chars: c.chars,
      })),
    };
    return fresh;
  }, [rustLayout, tab.path, tab.text, chunkChars, textLines, focusedChunkIdx]);

  const lineRanges = useMemo(() => {
    if (
      rustLayout &&
      rustLayout.path === tab.path &&
      rustLayout.text === tab.text &&
      rustLayout.pendingDiffs === tab.pendingDiffs
    ) {
      return rustLayout.lineRanges;
    }
    return [];
  }, [rustLayout, tab.path, tab.text, tab.pendingDiffs]);

  const diffByChunk = useMemo(() => {
    if (
      rustLayout &&
      rustLayout.path === tab.path &&
      rustLayout.text === tab.text &&
      rustLayout.pendingDiffs === tab.pendingDiffs
    ) {
      return rustLayout.diffByChunk;
    }
    return new Map<number, ChunkGroupDiffInfo>();
  }, [rustLayout, tab.path, tab.text, tab.pendingDiffs]);

  const onChunkChange = useCallback(
    (chunkIndex: number, startLine: number, endLine: number, value: string) => {
      const newChunkLines = value.split("\n");
      const delta = newChunkLines.length - (endLine - startLine + 1);
      const nextLines = textLines.slice();
      nextLines.splice(startLine, endLine - startLine + 1, ...newChunkLines);
      if (pinnedRangesRef.current) {
        pinnedRangesRef.current = {
          ...pinnedRangesRef.current,
          lineCount: nextLines.length,
          ranges:
            delta !== 0
              ? pinnedRangesRef.current.ranges.map((r, i) => {
                  if (i < chunkIndex) return r;
                  if (i === chunkIndex) {
                    return {
                      startLine: r.startLine,
                      endLine: r.startLine + newChunkLines.length - 1,
                      chars: r.chars,
                    };
                  }
                  return {
                    startLine: r.startLine + delta,
                    endLine: r.endLine + delta,
                    chars: r.chars,
                  };
                })
              : pinnedRangesRef.current.ranges,
        };
      }
      applyText(nextLines.join(eol));
    },
    [applyText, textLines, eol],
  );

  // Find overlay support
  const findOpen = useReaderFind((s) => s.open);
  const findQuery = useReaderFind((s) => s.query);
  const matchCase = useReaderFind((s) => s.matchCase);
  const matchIndex = useReaderFind((s) => s.matchIndex);
  const navEpoch = useReaderFind((s) => s.navEpoch);
  const findMatches = useReaderFind((s) => s.matches);

  const findOverlay = useMemo(() => {
    if (!findOpen || !findQuery.trim()) {
      return {
        located: [] as Array<{
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
      start: lineColOf(tab.text, range.start),
      end: lineColOf(tab.text, range.end),
    }));
    return { located, activeIndex, scrollToIndex };
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

  // Restore & track reading viewport progress
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const total = textLines.length;
    let restoring = false;
    let touched = false;
    let restoreRaf = 0;
    const stopRestore = () => {
      restoring = false;
      if (restoreRaf) cancelAnimationFrame(restoreRaf);
      restoreRaf = 0;
    };

    if (hasReaderViewport(tab.path)) {
      el.scrollTop = readerViewportTop(tab.path);
    } else {
      const savedLine = readerSavedLine(tab.path, total);
      if (savedLine != null) {
        restoring = true;
        const until = performance.now() + 1200;
        const tick = () => {
          restoreRaf = 0;
          if (!restoring) return;
          let host: HTMLElement | null = null;
          for (const node of Array.from(el.children)) {
            const child = node as HTMLElement;
            const s = Number(child.dataset.segStart);
            const e = Number(child.dataset.segEnd);
            if (savedLine >= s && savedLine <= e) {
              host = child;
              break;
            }
          }
          if (host) {
            const s = Number(host.dataset.segStart);
            const row = host.querySelectorAll(".cm-line")[savedLine - s];
            if (row) {
              const delta = row.getBoundingClientRect().top - el.getBoundingClientRect().top;
              if (Math.abs(delta) > 0.5) el.scrollTop += delta;
            } else {
              el.scrollTop += host.getBoundingClientRect().top - el.getBoundingClientRect().top;
            }
          }
          if (performance.now() < until) {
            restoreRaf = requestAnimationFrame(tick);
          } else {
            restoring = false;
            scrollTopRef.current = el.scrollTop;
          }
        };
        tick();
      }
    }
    scrollTopRef.current = el.scrollTop;

    const recordLine = () => {
      if (restoring) return;
      touched = true;
      const top = el.getBoundingClientRect().top;
      for (const node of Array.from(el.children)) {
        const child = node as HTMLElement;
        const rect = child.getBoundingClientRect();
        if (rect.bottom <= top + 1) continue;
        const start = Number(child.dataset.segStart);
        if (!Number.isFinite(start)) return;
        let line = start;
        const rows = child.querySelectorAll(".cm-line");
        for (let k = 0; k < rows.length; k += 1) {
          if (rows[k].getBoundingClientRect().bottom > top + 1) {
            line = start + k;
            break;
          }
        }
        rememberReaderLine(tab.path, line, Math.max(1, splitDocumentLines(findTextRef.current).lines.length));
        return;
      }
    };

    const onScroll = () => {
      scrollTopRef.current = el.scrollTop;
      if (restoring) return;
      rememberReaderViewport(tab.path, el.scrollTop);
      recordLine();
    };
    const onUser = () => stopRestore();
    el.addEventListener("scroll", onScroll, { passive: true });
    el.addEventListener("wheel", onUser, { passive: true });
    el.addEventListener("pointerdown", onUser);
    return () => {
      stopRestore();
      if (touched) {
        rememberReaderViewport(tab.path, el.scrollTop);
        recordLine();
      }
      flushReaderProgress();
      el.removeEventListener("scroll", onScroll);
      el.removeEventListener("wheel", onUser);
      el.removeEventListener("pointerdown", onUser);
    };
  }, [tab.path, textLines.length]);

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
      const row = host!.querySelectorAll(".cm-line")[localLine];
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

  const estimateChunkHeight = (startLine: number, endLine: number) => {
    let total = 34; // header height
    for (let i = startLine; i <= endLine; i += 1) {
      total += estimateLineHeight(textLines[i]?.length ?? 0, metrics);
    }
    return total;
  };

  const showBarForChunk = useCallback(
    (chunkIdx: number | null, hunkIdx?: number) => {
      if (hideBarTimerRef.current) {
        clearTimeout(hideBarTimerRef.current);
        hideBarTimerRef.current = null;
      }
      setHoveredChunkIdx(chunkIdx);
      if (chunkIdx != null && hunkIdx != null && hunkIdx >= 0) {
        onActiveHunkChange?.(hunkIdx);
      }
    },
    [onActiveHunkChange],
  );

  const scheduleHideBar = useCallback(() => {
    if (hideBarTimerRef.current) clearTimeout(hideBarTimerRef.current);
    hideBarTimerRef.current = setTimeout(() => setHoveredChunkIdx(null), DIFF_BAR_HIDE_MS);
  }, []);

  const navigateHunk = useCallback(
    (direction: -1 | 1) => {
      if (lineRanges.length === 0) return;
      const cur = activeHunkIndex != null && activeHunkIndex >= 0 ? activeHunkIndex : 0;
      const nextIdx = Math.max(0, Math.min(cur + direction, lineRanges.length - 1));
      const next = lineRanges[nextIdx];
      if (!next) return;
      onActiveHunkChange?.(nextIdx);
      hunkRefs.current.get(next.blockId)?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    },
    [activeHunkIndex, lineRanges, onActiveHunkChange],
  );

  useEffect(() => {
    if (activeHunkIndex == null) return;
    if (activeHunkIndex < 0) {
      lastScrolledHunkRef.current = -1;
      return;
    }
    const range = lineRanges[activeHunkIndex];
    if (!range) return;
    if (lastScrolledHunkRef.current === null) {
      lastScrolledHunkRef.current = activeHunkIndex;
      return;
    }
    const changed = lastScrolledHunkRef.current !== activeHunkIndex;
    lastScrolledHunkRef.current = activeHunkIndex;
    if (!changed) return;
    hunkRefs.current.get(range.blockId)?.scrollIntoView({
      block: "nearest",
      behavior: "smooth",
    });
  }, [activeHunkIndex, lineRanges]);

  const handleChunkFocusChange = useCallback((chunkIndex: number, focused: boolean) => {
    setFocusedChunkIdx((prev) => {
      if (focused) return chunkIndex;
      return prev === chunkIndex ? null : prev;
    });
  }, []);

  const registerHunkRef = useCallback((blockIds: string[], el: HTMLElement | null) => {
    for (const id of blockIds) {
      if (el) hunkRefs.current.set(id, el);
      else hunkRefs.current.delete(id);
    }
  }, []);

  return (
    <div className="reader-editor-wrap reader-editor-wrap--chunked reader-editor-wrap--codemirror">
      <div className="reader-editor-main" onMouseLeave={scheduleHideBar}>
        <div ref={setScrollNode} className="reader-editor-scroll">
          {chunks.map((chunk) => {
            const find = findForSpan(chunk.startLine, chunk.endLine);
            const diffInfo = diffByChunk.get(chunk.index) ?? null;
            const showBar =
              diffInfo != null &&
              diffInfo.range != null &&
              diffInfo.hunkIndex >= 0 &&
              hoveredChunkIdx === chunk.index;

            return (
              <ChunkGroupItem
                key={`chunk-${chunk.index}`}
                chunk={chunk}
                tab={tab}
                scrollEl={scrollEl}
                estimatedHeight={estimateChunkHeight(chunk.startLine, chunk.endLine)}
                findRanges={find.ranges}
                findActiveIndex={find.activeIndex}
                diffInfo={diffInfo}
                showBar={showBar}
                hunkTotal={lineRanges.length}
                rewriteRunning={!!rewriteRunning}
                hasRewriteHandler={!!onRewriteChunk}
                onRewriteChunk={onRewriteChunk}
                onChunkChange={onChunkChange}
                onFocusChange={handleChunkFocusChange}
                onShowBar={showBarForChunk}
                onScheduleHideBar={scheduleHideBar}
                onNavigateHunk={navigateHunk}
                registerHunkRef={registerHunkRef}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}
