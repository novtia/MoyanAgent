import { diffChars, diffLines, type Change } from "diff";
import { formatParagraphNumber } from "../store/reader";

/** Strip optional `[P123]` prefix from agent paragraph snippets. */
function stripParagraphLabelPrefix(s: string): string {
  const trimmed = s.trimStart();
  if (!trimmed.startsWith("[P")) return s;
  const rest = trimmed.slice(2);
  const closeIdx = rest.indexOf("]");
  if (closeIdx < 0) return s;
  const digits = rest.slice(0, closeIdx);
  if (!/^\d+$/.test(digits)) return s;
  return rest.slice(closeIdx + 1).trimStart();
}

/** Normalize one diff line for semantic equality (labels, NFC, line endings). */
export function normalizeDiffLine(s: string): string {
  return stripParagraphLabelPrefix(s).normalize("NFC").replace(/\r/g, "");
}

/** Normalize multi-line diff text for semantic equality. */
export function normalizeDiffText(s: string): string {
  return s
    .replace(/\r\n/g, "\n")
    .replace(/\r/g, "\n")
    .split("\n")
    .map((line) => normalizeDiffLine(line))
    .join("\n");
}

export function isDiffTextEqual(a: string, b: string): boolean {
  return normalizeDiffText(a) === normalizeDiffText(b);
}

export type DiffRow =
  | { kind: "equal"; text: string }
  | { kind: "delete"; text: string }
  | { kind: "insert"; text: string }
  | { kind: "replace"; oldText: string; newText: string };

export type DocDiffSegment =
  | { kind: "context"; lines: string[]; startLine: number }
  | {
      kind: "hunk";
      blockId: string;
      rows: DiffRow[];
      paragraphNumber?: number;
      startLine: number;
      skip: number;
    };

/** Split diff line values into individual rows (without trailing empty from final newline). */
function splitDiffValue(value: string): string[] {
  if (value === "") return [""];
  const parts = value.split("\n");
  if (parts.length > 1 && parts[parts.length - 1] === "") {
    parts.pop();
  }
  return parts;
}

/** Merge consecutive remove+add from diffLines into replace rows. */
export function buildDiffRows(oldText: string, newText: string): DiffRow[] {
  if (isDiffTextEqual(oldText, newText)) {
    const lines = normalizeDiffText(newText).split("\n");
    if (lines.length === 1 && lines[0] === "") return [];
    return lines.map((text) => ({ kind: "equal" as const, text }));
  }

  const parts = diffLines(oldText, newText);
  const rows: DiffRow[] = [];
  let i = 0;
  while (i < parts.length) {
    const cur = parts[i];
    const next = parts[i + 1];
    if (cur.removed && next?.added) {
      const oldLines = splitDiffValue(cur.value);
      const newLines = splitDiffValue(next.value);
      const n = Math.max(oldLines.length, newLines.length);
      for (let j = 0; j < n; j += 1) {
        const o = oldLines[j];
        const ne = newLines[j];
        if (o !== undefined && ne !== undefined) {
          if (o === ne || normalizeDiffLine(o) === normalizeDiffLine(ne)) {
            rows.push({ kind: "equal", text: ne });
          } else {
            rows.push({ kind: "replace", oldText: o, newText: ne });
          }
        } else if (o !== undefined) {
          rows.push({ kind: "delete", text: o });
        } else if (ne !== undefined) {
          rows.push({ kind: "insert", text: ne });
        }
      }
      i += 2;
    } else if (cur.removed) {
      for (const line of splitDiffValue(cur.value)) {
        rows.push({ kind: "delete", text: line });
      }
      i += 1;
    } else if (cur.added) {
      for (const line of splitDiffValue(cur.value)) {
        rows.push({ kind: "insert", text: line });
      }
      i += 1;
    } else {
      for (const line of splitDiffValue(cur.value)) {
        rows.push({ kind: "equal", text: line });
      }
      i += 1;
    }
  }
  return rows;
}

/** Keep only hunks around changes plus a little context (IDE-style). */
export function foldDiffRows(rows: DiffRow[], contextLines = 3): DiffRow[] {
  const changed = new Set<number>();
  rows.forEach((r, idx) => {
    if (r.kind !== "equal") changed.add(idx);
  });
  if (changed.size === 0) return rows;

  const keep = new Set<number>();
  for (const idx of changed) {
    for (let k = idx - contextLines; k <= idx + contextLines; k += 1) {
      if (k >= 0 && k < rows.length) keep.add(k);
    }
  }

  const sorted = [...keep].sort((a, b) => a - b);
  const out: DiffRow[] = [];
  let prev = -1;
  for (const idx of sorted) {
    if (prev >= 0 && idx > prev + 1) {
      out.push({ kind: "equal", text: "…" });
    }
    out.push(rows[idx]);
    prev = idx;
  }
  return out;
}

function findLineSubsequence(haystack: string[], needle: string[]): number {
  if (needle.length === 0) return -1;
  for (let i = 0; i <= haystack.length - needle.length; i += 1) {
    let ok = true;
    for (let j = 0; j < needle.length; j += 1) {
      if (haystack[i + j] !== needle[j]) {
        ok = false;
        break;
      }
    }
    if (ok) return i;
  }
  return -1;
}

/** Character index of `needle` in `haystack` (exact, then NFC-normalized). */
function findSnippetCharIndex(haystack: string, needle: string): number {
  if (!needle) return -1;
  const exact = haystack.indexOf(needle);
  if (exact >= 0) return exact;
  const normHay = normalizeDiffText(haystack);
  const normNeedle = normalizeDiffText(needle);
  if (!normNeedle) return -1;
  const nIdx = normHay.indexOf(normNeedle);
  if (nIdx < 0) return -1;
  // When normalization did not change length, indices align.
  if (normHay.length === haystack.length) return nIdx;
  // Otherwise re-find the original needle near the normalized hit.
  const window = haystack.slice(Math.max(0, nIdx - needle.length), nIdx + needle.length * 2);
  const local = window.indexOf(needle);
  if (local >= 0) return Math.max(0, nIdx - needle.length) + local;
  return nIdx;
}

function charRangeToLines(
  text: string,
  start: number,
  end: number,
): { startLine: number; endLine: number } {
  const safeStart = Math.max(0, Math.min(start, text.length));
  const safeEnd = Math.max(safeStart, Math.min(end, text.length));
  const startLine = text.slice(0, safeStart).split("\n").length - 1;
  // A range ending exactly on a newline belongs to the previous line.
  const endPos = safeEnd > safeStart && text[safeEnd - 1] === "\n" ? safeEnd - 1 : safeEnd;
  const endLine = text.slice(0, endPos).split("\n").length - 1;
  return { startLine, endLine: Math.max(startLine, endLine) };
}

/**
 * Map a character range in `blocks[fromIndex].textAfter` forward through later
 * Edit replacements so it lands in the final document (last textAfter / tabText).
 */
function mapCharRangeThroughLaterEdits(
  start: number,
  end: number,
  fromIndex: number,
  blocks: DocumentDiffBlockInput[],
): { start: number; end: number } {
  let s = start;
  let e = end;
  let text = blocks[fromIndex]?.textAfter ?? "";

  for (let j = fromIndex + 1; j < blocks.length; j += 1) {
    const b = blocks[j];
    let at = findSnippetCharIndex(text, b.before);
    if (at < 0 && b.textBefore) {
      // text should equal this block's textBefore for sequential edits
      at = findSnippetCharIndex(b.textBefore, b.before);
      text = b.textBefore;
    }
    if (at < 0) {
      text = b.textAfter;
      continue;
    }

    const beforeLen = b.before.length;
    const afterLen = b.after.length;
    const delta = afterLen - beforeLen;
    const editEnd = at + beforeLen;

    if (e <= at) {
      // entirely before this edit
    } else if (s >= editEnd) {
      s += delta;
      e += delta;
    } else {
      // Overlap: union with the replacement span
      const newStart = Math.min(s, at);
      let newEnd = at + afterLen;
      if (e > editEnd) newEnd = Math.max(newEnd, e + delta);
      s = newStart;
      e = newEnd;
    }
    text = b.textAfter;
  }

  return { start: s, end: e };
}

/**
 * Locate where a pending Edit should sit in the current `tabText`.
 * Uses character-level snippet search (Edit old/new_string need not be full lines)
 * and maps earlier hunks through later edits when their `after` was superseded.
 */
function placePendingBlockInTab(
  tabText: string,
  blocks: DocumentDiffBlockInput[],
  index: number,
): { startLine: number; endLine: number } {
  const block = blocks[index];
  const tabLines = tabText.split("\n");

  const clampToTab = (start: number, end: number) =>
    charRangeToLines(
      tabText,
      Math.max(0, Math.min(start, tabText.length)),
      Math.max(0, Math.min(end, tabText.length)),
    );

  // 1) `after` still present in the live document (common for the latest edit).
  if (block.after.trim()) {
    const liveIdx = findSnippetCharIndex(tabText, block.after);
    if (liveIdx >= 0) {
      return clampToTab(liveIdx, liveIdx + block.after.length);
    }
  }

  // 2) Find `after` in this edit's textAfter, then map through later edits.
  if (block.after.trim() && block.textAfter) {
    const afterIdx = findSnippetCharIndex(block.textAfter, block.after);
    if (afterIdx >= 0) {
      const mapped = mapCharRangeThroughLaterEdits(
        afterIdx,
        afterIdx + block.after.length,
        index,
        blocks,
      );
      return clampToTab(mapped.start, mapped.end);
    }
  }

  // 3) `before` in textBefore → locate the matching `after` start, map forward.
  if (block.before.trim() && block.textBefore && block.textAfter) {
    const beforeIdx = findSnippetCharIndex(block.textBefore, block.before);
    if (beforeIdx >= 0) {
      // In a successful Edit, `after` starts at the same offset as `before`.
      const afterStart = Math.min(beforeIdx, block.textAfter.length);
      const afterEnd = Math.min(
        afterStart + block.after.length,
        block.textAfter.length,
      );
      const mapped = mapCharRangeThroughLaterEdits(
        afterStart,
        afterEnd,
        index,
        blocks,
      );
      return clampToTab(mapped.start, mapped.end);
    }
  }

  // 4) Full-line subsequence / paragraph fallbacks (legacy).
  const newLines = block.after.split("\n");
  const oldLines = block.before.split("\n");
  let start = -1;
  if (block.paragraphNumber != null) {
    start =
      block.before.trim() === "" && block.after.trim() !== ""
        ? lineAfterParagraph(tabText, block.paragraphNumber)
        : paragraphStartLine(tabText, block.paragraphNumber);
  }
  if (start < 0 && block.after.trim()) {
    start = findLineSubsequence(tabLines, newLines);
  }
  if (start < 0 && block.before.trim()) {
    start = findLineSubsequence(tabLines, oldLines);
  }
  if (start < 0) start = 0;
  const lineCount = block.after.trim()
    ? Math.max(newLines.length, 1)
    : Math.max(oldLines.length, 1);
  const endLine = Math.min(tabLines.length - 1, start + lineCount - 1);
  return { startLine: start, endLine: Math.max(start, endLine) };
}

/** Line index where paragraph `oneBased` begins (0-based; one line = one paragraph). */
export function paragraphStartLine(_text: string, oneBased: number): number {
  return Math.max(0, oneBased - 1);
}

/** Line index where content inserted after paragraph `afterOneBased` begins (0-based). */
export function lineAfterParagraph(text: string, afterOneBased: number): number {
  const lineCount = text.split("\n").length;
  return Math.min(lineCount, afterOneBased);
}

export interface DocumentDiffBlockInput {
  id: string;
  before: string;
  after: string;
  textBefore: string;
  textAfter: string;
  paragraphNumber?: number;
}

/**
 * Merge full document text with inline diff hunks (one hunk per Edit).
 * Unchanged regions render as plain lines; each Edit renders colored −/+ rows in place.
 */
export function buildDocumentDiffSegments(
  tabText: string,
  blocks: DocumentDiffBlockInput[],
): DocDiffSegment[] {
  if (blocks.length === 0) {
    const lines = tabText.split("\n");
    return lines.length > 0 ? [{ kind: "context", lines, startLine: 0 }] : [];
  }

  const tabLines = tabText.split("\n");

  type HunkPlacement = {
    blockId: string;
    rows: DiffRow[];
    paragraphNumber?: number;
    start: number;
    skip: number;
  };

  const hunks: HunkPlacement[] = blocks.map((block, index) => {
    const rows = buildDiffRows(block.before, block.after);
    const { startLine: start, endLine } = placePendingBlockInTab(tabText, blocks, index);
    const skip = Math.max(1, endLine - start + 1);
    return {
      blockId: block.id,
      rows,
      paragraphNumber: block.paragraphNumber,
      start,
      skip,
    };
  });

  hunks.sort((a, b) => a.start - b.start);

  const segments: DocDiffSegment[] = [];
  let cursor = 0;
  let i = 0;

  while (i < hunks.length) {
    const group = [hunks[i]];
    let groupStart = hunks[i].start;
    let groupEnd = hunks[i].start + hunks[i].skip - 1;
    let j = i + 1;
    while (j < hunks.length && hunks[j].start <= groupEnd) {
      group.push(hunks[j]);
      groupEnd = Math.max(groupEnd, hunks[j].start + hunks[j].skip - 1);
      j += 1;
    }

    if (groupStart > cursor) {
      segments.push({
        kind: "context",
        lines: tabLines.slice(cursor, groupStart),
        startLine: cursor,
      });
    }
    for (const hunk of group) {
      segments.push({
        kind: "hunk",
        blockId: hunk.blockId,
        rows: hunk.rows,
        paragraphNumber: hunk.paragraphNumber,
        startLine: groupStart,
        skip: hunk.skip,
      });
    }
    cursor = groupEnd + 1;
    i = j;
  }

  if (cursor < tabLines.length) {
    segments.push({ kind: "context", lines: tabLines.slice(cursor), startLine: cursor });
  }

  return segments;
}

export interface PendingDiffLineRange {
  blockId: string;
  startLine: number;
  endLine: number;
  before: string;
  after: string;
}

/** Map each pending Edit to 0-based line range in current `tabText`. */
export function buildPendingDiffLineRanges(
  tabText: string,
  blocks: DocumentDiffBlockInput[],
): PendingDiffLineRange[] {
  return blocks.map((block, index) => {
    const { startLine, endLine } = placePendingBlockInTab(tabText, blocks, index);
    return {
      blockId: block.id,
      startLine,
      endLine,
      before: block.before,
      after: block.after,
    };
  });
}

export function pendingDiffRangeAtLine(
  lineIdx: number,
  ranges: PendingDiffLineRange[],
): PendingDiffLineRange | undefined {
  return ranges.find((r) => lineIdx >= r.startLine && lineIdx <= r.endLine);
}

export type EditorDisplaySegment =
  | { kind: "context"; tabStart: number; tabEnd: number }
  | {
      kind: "hunk";
      blockId: string;
      before: string;
      after: string;
      tabStart: number;
      tabEnd: number;
      paragraphNumber?: number;
    };

/** Split tab.text into context + hunk (red delete / green insert) segments. */
export function buildEditorDisplaySegments(
  tabText: string,
  blocks: DocumentDiffBlockInput[],
): EditorDisplaySegment[] {
  const ranges = buildPendingDiffLineRanges(tabText, blocks);
  if (ranges.length === 0) {
    const n = tabText.split("\n").length;
    return n > 0 ? [{ kind: "context", tabStart: 0, tabEnd: n - 1 }] : [];
  }

  // Preserve chronological order within the same start line so sequential
  // edits on one region stack in application order.
  const order = new Map(blocks.map((b, i) => [b.id, i]));
  const sorted = [...ranges].sort((a, b) => {
    if (a.startLine !== b.startLine) return a.startLine - b.startLine;
    return (order.get(a.blockId) ?? 0) - (order.get(b.blockId) ?? 0);
  });

  const segments: EditorDisplaySegment[] = [];
  const totalLines = tabText.split("\n").length;
  let cursor = 0;
  let i = 0;

  while (i < sorted.length) {
    // Group overlapping / nested ranges so final lines are only consumed once.
    const group = [sorted[i]];
    let groupStart = sorted[i].startLine;
    let groupEnd = sorted[i].endLine;
    let j = i + 1;
    while (j < sorted.length && sorted[j].startLine <= groupEnd) {
      group.push(sorted[j]);
      groupEnd = Math.max(groupEnd, sorted[j].endLine);
      j += 1;
    }

    if (groupStart > cursor) {
      segments.push({
        kind: "context",
        tabStart: cursor,
        tabEnd: groupStart - 1,
      });
    }

    for (const range of group) {
      const block = blocks.find((b) => b.id === range.blockId);
      segments.push({
        kind: "hunk",
        blockId: range.blockId,
        before: range.before,
        after: range.after,
        tabStart: range.startLine,
        tabEnd: range.endLine,
        paragraphNumber: block?.paragraphNumber,
      });
    }

    cursor = groupEnd + 1;
    i = j;
  }

  if (cursor < totalLines) {
    segments.push({ kind: "context", tabStart: cursor, tabEnd: totalLines - 1 });
  }

  return segments;
}

/** Replace inclusive line range in full document text. */
export function replaceTabLineRange(
  text: string,
  tabStart: number,
  tabEnd: number,
  replacement: string,
): string {
  const lines = text.split("\n");
  const newLines = replacement.split("\n");
  lines.splice(tabStart, tabEnd - tabStart + 1, ...newLines);
  return lines.join("\n");
}

export function sliceTabLines(text: string, tabStart: number, tabEnd: number): string {
  return text.split("\n").slice(tabStart, tabEnd + 1).join("\n");
}

/** Per-line before/after snippet inside a pending Edit range. */
export function lineSnippetsInRange(
  lineIdx: number,
  range: PendingDiffLineRange,
  currentLine: string,
): { oldLine: string; newLine: string } {
  const beforeLines = range.before.split("\n");
  const afterLines = range.after.split("\n");
  const offset = lineIdx - range.startLine;
  return {
    oldLine: beforeLines[offset] ?? "",
    newLine: afterLines[offset] ?? currentLine,
  };
}

/** Backdrop line: char-level insert highlights (text visible; textarea sits on top transparent). */
export function EditorLineHighlight({
  oldLine,
  newLine,
}: {
  oldLine: string;
  newLine: string;
}) {
  if (oldLine === newLine || normalizeDiffLine(oldLine) === normalizeDiffLine(newLine)) {
    return (
      <span className="reader-editor-backdrop-text reader-editor-backdrop-text--changed">
        {newLine || "\u00a0"}
      </span>
    );
  }
  const parts = diffChars(oldLine, newLine);
  return (
    <span className="reader-editor-backdrop-text reader-editor-backdrop-text--changed">
      {parts.map((part, i) => {
        if (part.removed) return null;
        const cls = part.added ? "reader-diff-char is-added" : "";
        return (
          <span key={i} className={cls}>
            {part.value}
          </span>
        );
      })}
      {!newLine && "\u00a0"}
    </span>
  );
}

function ParaGutter({ label }: { label?: number | null }) {
  return (
    <span className="reader-diff-para" aria-hidden={label == null}>
      {label != null ? formatParagraphNumber(label) : ""}
    </span>
  );
}

function CharSpans({
  parts,
  side,
}: {
  parts: Change[];
  side: "old" | "new";
}) {
  return (
    <>
      {parts.map((part, i) => {
        if (side === "old" && part.added) return null;
        if (side === "new" && part.removed) return null;
        let cls = "reader-diff-char";
        if (part.added) cls += " is-added";
        if (part.removed) cls += " is-removed";
        return (
          <span key={i} className={cls}>
            {part.value}
          </span>
        );
      })}
    </>
  );
}

function ReplaceLinePair({
  oldText,
  newText,
  paragraphLabel,
  insertParagraphLabel,
}: {
  oldText: string;
  newText: string;
  paragraphLabel?: number | null;
  insertParagraphLabel?: number | null;
}) {
  if (normalizeDiffLine(oldText) === normalizeDiffLine(newText)) {
    return (
      <div className="reader-diff-line is-context">
        <ParaGutter label={paragraphLabel} />
        <span className="reader-diff-gutter" aria-hidden />
        <span className="reader-diff-text">{newText || " "}</span>
      </div>
    );
  }
  const parts = diffChars(oldText, newText);
  return (
    <>
      <div className="reader-diff-line is-delete" aria-label="removed">
        <ParaGutter label={paragraphLabel} />
        <span className="reader-diff-gutter">−</span>
        <span className="reader-diff-text">
          <CharSpans parts={parts} side="old" />
        </span>
      </div>
      <div className="reader-diff-line is-insert" aria-label="added">
        <ParaGutter label={insertParagraphLabel} />
        <span className="reader-diff-gutter">+</span>
        <span className="reader-diff-text">
          <CharSpans parts={parts} side="new" />
        </span>
      </div>
    </>
  );
}

export function DiffRowView({
  row,
  idx,
  paragraphLabel,
  insertParagraphLabel,
}: {
  row: DiffRow;
  idx: number;
  paragraphLabel?: number | null;
  insertParagraphLabel?: number | null;
}) {
  if (row.kind === "equal") {
    return (
      <div key={idx} className="reader-diff-line is-context">
        <ParaGutter label={paragraphLabel} />
        <span className="reader-diff-gutter" aria-hidden />
        <span className="reader-diff-text">{row.text || " "}</span>
      </div>
    );
  }
  if (row.kind === "delete") {
    return (
      <div key={idx} className="reader-diff-line is-delete">
        <ParaGutter label={paragraphLabel} />
        <span className="reader-diff-gutter">−</span>
        <span className="reader-diff-text">{row.text || " "}</span>
      </div>
    );
  }
  if (row.kind === "insert") {
    return (
      <div key={idx} className="reader-diff-line is-insert">
        <ParaGutter label={paragraphLabel} />
        <span className="reader-diff-gutter">+</span>
        <span className="reader-diff-text">{row.text || " "}</span>
      </div>
    );
  }
  return (
    <ReplaceLinePair
      key={idx}
      oldText={row.oldText}
      newText={row.newText}
      paragraphLabel={paragraphLabel}
      insertParagraphLabel={insertParagraphLabel}
    />
  );
}

export function InlineDiffCode({
  oldText,
  newText,
  maxLinesBeforeFold = 80,
}: {
  oldText: string;
  newText: string;
  maxLinesBeforeFold?: number;
}) {
  let rows = buildDiffRows(oldText, newText);
  if (rows.length > maxLinesBeforeFold) {
    rows = foldDiffRows(rows, 3);
  }

  return (
    <pre className="reader-diff-code">
      <code>
        {rows.map((row, idx) => (
          <DiffRowView key={idx} row={row} idx={idx} />
        ))}
      </code>
    </pre>
  );
}
