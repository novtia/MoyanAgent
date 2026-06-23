import { textareaContentWidth } from "./readerGutter";

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

function applyMirrorTypography(el: HTMLTextAreaElement, mirror: HTMLDivElement, width: number) {
  const cs = getComputedStyle(el);
  mirror.style.position = "absolute";
  mirror.style.visibility = "hidden";
  mirror.style.pointerEvents = "none";
  mirror.style.left = "-9999px";
  mirror.style.top = "0";
  mirror.style.width = `${width}px`;
  mirror.style.fontFamily = cs.fontFamily;
  mirror.style.fontSize = cs.fontSize;
  mirror.style.fontWeight = cs.fontWeight;
  mirror.style.lineHeight = cs.lineHeight;
  mirror.style.letterSpacing = cs.letterSpacing;
  mirror.style.whiteSpace = "pre-wrap";
  mirror.style.wordBreak = "break-word";
  mirror.style.overflowWrap = "break-word";
  mirror.style.boxSizing = "border-box";
}

/** Pixel offset of a character index inside textarea content (handles wrapping). */
export function measureTextareaCharOffsetTop(
  el: HTMLTextAreaElement,
  index: number,
): number {
  const contentWidth = textareaContentWidth(el);
  if (contentWidth <= 0) return 0;

  const clamped = Math.max(0, Math.min(index, el.value.length));
  const mirror = document.createElement("div");
  applyMirrorTypography(el, mirror, contentWidth);

  const before = el.value.slice(0, clamped);
  const after = el.value.slice(clamped);
  const marker = document.createElement("span");
  marker.textContent = "\u200b";
  mirror.append(document.createTextNode(before), marker, document.createTextNode(after));

  document.body.appendChild(mirror);
  const top = marker.offsetTop;
  document.body.removeChild(mirror);
  return top;
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

/** Scroll textarea so the given character index is visible. */
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
