import { countWords } from "../store/reader";

export interface ReaderParagraphChunk {
  /** 0-based index of the chunk in the document. */
  index: number;
  /** 0-based inclusive start line (paragraph - 1). */
  startLine: number;
  /** 0-based inclusive end line (paragraph - 1). */
  endLine: number;
  /** Word count (non-whitespace chars) of lines in this chunk. */
  chars: number;
  /** 1-based paragraph labels for each line in `[startLine, endLine]`. */
  labels: number[];
  /** Joined LF text of lines in `[startLine, endLine]`. */
  text: string;
}

/** Split raw document text into LF-normalized lines and detect its line ending. */
export function splitDocumentLines(text: string): {
  lines: string[];
  eol: "\r\n" | "\n";
} {
  const eol = text.includes("\r\n") ? "\r\n" : "\n";
  const lines = text
    .split("\n")
    .map((line) => (line.endsWith("\r") ? line.slice(0, -1) : line));
  return { lines, eol };
}

/**
 * Split document lines into chunks strictly along paragraph boundaries (`\n`),
 * never breaking inside a paragraph or sentence.
 *
 * If a single paragraph alone exceeds `maxChars`, it is kept whole in its own
 * chunk. Empty lines have 0 words and keep their original 1-based paragraph
 * numbers intact.
 */
export function splitLinesIntoParagraphChunks(
  lines: string[],
  maxChars: number,
  protectedSpans?: Array<{ startLine: number; endLine: number }>,
): ReaderParagraphChunk[] {
  const limit = Math.max(1, Math.floor(Number.isFinite(maxChars) ? maxChars : 700));
  if (lines.length === 0) return [];

  const isInsideProtectedSpan = (prevLine: number, nextLine: number): boolean => {
    if (!protectedSpans || protectedSpans.length === 0) return false;
    return protectedSpans.some((s) => prevLine >= s.startLine && nextLine <= s.endLine);
  };

  const chunks: ReaderParagraphChunk[] = [];
  let startLine = 0;
  let accChars = countWords(lines[0] ?? "");

  const pushChunk = (from: number, to: number, chars: number) => {
    const labels: number[] = [];
    for (let k = from; k <= to; k += 1) labels.push(k + 1);
    chunks.push({
      index: chunks.length,
      startLine: from,
      endLine: to,
      chars,
      labels,
      text: lines.slice(from, to + 1).join("\n"),
    });
  };

  for (let i = 1; i < lines.length; i += 1) {
    const w = countWords(lines[i] ?? "");
    if (accChars + w <= limit || isInsideProtectedSpan(i - 1, i)) {
      accChars += w;
    } else {
      pushChunk(startLine, i - 1, accChars);
      startLine = i;
      accChars = w;
    }
  }
  pushChunk(startLine, lines.length - 1, accChars);

  return chunks;
}

/** Rebuild chunk objects from pinned `[startLine, endLine]` ranges. */
export function buildChunksFromRanges(
  lines: string[],
  ranges: Array<{ startLine: number; endLine: number; chars?: number }>,
): ReaderParagraphChunk[] {
  const last = Math.max(0, lines.length - 1);
  return ranges.map((r, index) => {
    const startLine = Math.min(Math.max(0, r.startLine), last);
    const endLine = Math.min(Math.max(startLine, r.endLine), last);
    const slice = lines.slice(startLine, endLine + 1);
    const labels: number[] = [];
    let chars = r.chars ?? 0;
    const needCount = r.chars == null;
    for (let k = startLine; k <= endLine; k += 1) {
      labels.push(k + 1);
      if (needCount) chars += countWords(lines[k] ?? "");
    }
    return {
      index,
      startLine,
      endLine,
      chars,
      labels,
      text: slice.join("\n"),
    };
  });
}
