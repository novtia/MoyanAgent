import {
  measureTextareaCharOffsetTop,
  textareaContentWidth,
} from "./readerMirror";

export { measureTextareaCharOffsetTop, textareaContentWidth };

/** Collect UTF-8 text file paths under a project root (BFS, capped). */
export const PROJECT_SEARCH_FILE_CAP = 500;

const TEXT_EXTENSIONS = [
  ".txt",
  ".md",
  ".markdown",
  ".mdx",
  ".json",
  ".yaml",
  ".yml",
  ".csv",
  ".log",
  ".ini",
  ".toml",
  ".xml",
  ".html",
  ".css",
  ".js",
  ".ts",
  ".tsx",
  ".jsx",
  ".rs",
  ".py",
];

export function isSearchableTextFile(path: string): boolean {
  const lower = path.toLowerCase();
  return TEXT_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

export interface TextRange {
  start: number;
  end: number;
}

export function findInText(
  text: string,
  query: string,
  matchCase: boolean,
): TextRange[] {
  if (!query) return [];
  const haystack = matchCase ? text : text.toLowerCase();
  const needle = matchCase ? query : query.toLowerCase();
  const ranges: TextRange[] = [];
  let pos = 0;
  while (pos <= haystack.length) {
    const idx = haystack.indexOf(needle, pos);
    if (idx === -1) break;
    ranges.push({ start: idx, end: idx + query.length });
    pos = idx + Math.max(needle.length, 1);
  }
  return ranges;
}

export function lineColumnAt(text: string, index: number): { line: number; column: number } {
  const before = text.slice(0, index);
  const lines = before.split("\n");
  return {
    line: lines.length,
    column: (lines[lines.length - 1]?.length ?? 0) + 1,
  };
}

export function replaceRange(
  text: string,
  start: number,
  end: number,
  replacement: string,
): string {
  return text.slice(0, start) + replacement + text.slice(end);
}

/** Resolve a scroll target for a global find match against the live tab text. */
export function resolveFindScrollIndex(
  tabText: string,
  query: string,
  matchCase: boolean,
  match: { start: number; end: number },
  fileMatches: Array<{ start: number; end: number }>,
): number | null {
  const localRanges = findInText(tabText, query, matchCase);
  if (localRanges.length === 0) return null;

  const exact = localRanges.find(
    (r) => r.start === match.start && r.end === match.end,
  );
  if (exact) return exact.start;

  const ord = fileMatches.findIndex(
    (m) => m.start === match.start && m.end === match.end,
  );
  if (ord >= 0 && localRanges[ord]) return localRanges[ord].start;

  return localRanges[0]?.start ?? null;
}

/** Scroll the reader surface (outer scroll container) so the given character
 *  index is visible. `textareaEl` provides the value + typography; the actual
 *  scroll is applied to `scrollEl`. `padY` is the row-level top padding so the
 *  caret coordinate (which is content-relative) maps to the scroll coordinate. */
export function scrollReaderSurfaceToIndex(
  scrollEl: HTMLElement,
  textareaEl: HTMLTextAreaElement,
  index: number,
  padY: number,
) {
  const clamped = Math.max(0, Math.min(index, textareaEl.value.length));
  const charTop = measureTextareaCharOffsetTop(textareaEl, clamped);
  const target = charTop + padY - Math.max(scrollEl.clientHeight / 3, 48);
  const maxScroll = Math.max(0, scrollEl.scrollHeight - scrollEl.clientHeight);
  scrollEl.scrollTop = Math.max(0, Math.min(target, maxScroll));
}

/** @deprecated use scrollReaderSurfaceToIndex (single-scroll-container layout) */
export function scrollTextareaToIndex(el: HTMLTextAreaElement, index: number) {
  const clamped = Math.max(0, Math.min(index, el.value.length));
  const charTop = measureTextareaCharOffsetTop(el, clamped);
  const cs = getComputedStyle(el);
  const paddingTop = Number.parseFloat(cs.paddingTop) || 0;
  const target = charTop + paddingTop - Math.max(el.clientHeight / 3, 48);
  const maxScroll = Math.max(0, el.scrollHeight - el.clientHeight);
  el.scrollTop = Math.max(0, Math.min(target, maxScroll));
}

/** @deprecated use scrollTextareaToIndex */
export function scrollTextareaToSelection(el: HTMLTextAreaElement) {
  scrollTextareaToIndex(el, el.selectionStart);
}
