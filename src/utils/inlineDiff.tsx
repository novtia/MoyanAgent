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

  const hunks: HunkPlacement[] = blocks.map((block) => {
    const rows = buildDiffRows(block.before, block.after);
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
    if (start < 0) start = 0;

    const skip = block.after.trim()
      ? newLines.length
      : Math.max(oldLines.length, 1);
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

  for (const hunk of hunks) {
    const safeStart = Math.max(hunk.start, cursor);
    if (safeStart > cursor) {
      segments.push({ kind: "context", lines: tabLines.slice(cursor, safeStart), startLine: cursor });
    }
    segments.push({
      kind: "hunk",
      blockId: hunk.blockId,
      rows: hunk.rows,
      paragraphNumber: hunk.paragraphNumber,
      startLine: safeStart,
      skip: hunk.skip,
    });
    cursor = Math.max(cursor, safeStart + hunk.skip);
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
  const tabLines = tabText.split("\n");

  return blocks.map((block) => {
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
    if (start < 0) start = 0;

    const lineCount = block.after.trim()
      ? newLines.length
      : Math.max(oldLines.length, 1);
    const endLine = Math.min(tabLines.length - 1, start + lineCount - 1);

    return {
      blockId: block.id,
      startLine: start,
      endLine: Math.max(start, endLine),
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

  const sorted = [...ranges].sort((a, b) => a.startLine - b.startLine);
  const segments: EditorDisplaySegment[] = [];
  const totalLines = tabText.split("\n").length;
  let cursor = 0;

  for (const range of sorted) {
    const block = blocks.find((b) => b.id === range.blockId);
    if (range.startLine > cursor) {
      segments.push({
        kind: "context",
        tabStart: cursor,
        tabEnd: range.startLine - 1,
      });
    }
    segments.push({
      kind: "hunk",
      blockId: range.blockId,
      before: range.before,
      after: range.after,
      tabStart: range.startLine,
      tabEnd: range.endLine,
      paragraphNumber: block?.paragraphNumber,
    });
    cursor = range.endLine + 1;
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
