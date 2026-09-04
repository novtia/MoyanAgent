import type { KeyboardEvent } from "react";

/** Two full-width spaces — standard Chinese first-line paragraph indent. */
export const PARAGRAPH_INDENT = "　　";

const INDENT_LEN = PARAGRAPH_INDENT.length;

export interface ParagraphIndentChange {
  from: number;
  to: number;
  insert: string;
}

function lineIndexAt(text: string, pos: number): number {
  return text.slice(0, Math.max(0, pos)).split("\n").length - 1;
}

function lineStartOffsets(text: string): number[] {
  const lines = text.split("\n");
  const offsets: number[] = [];
  let at = 0;
  for (let i = 0; i < lines.length; i += 1) {
    offsets.push(at);
    at += lines[i].length + 1;
  }
  return offsets;
}

/**
 * Tab → prepend two full-width spaces to selected line(s).
 * Shift+Tab → remove leading indent when present.
 */
export function applyParagraphIndent(
  text: string,
  selectionStart: number,
  selectionEnd: number,
  outdent: boolean,
): { text: string; selectionStart: number; selectionEnd: number } | null {
  const lines = text.split("\n");
  if (lines.length === 0) return null;

  const startLine = lineIndexAt(text, selectionStart);
  const endLine =
    selectionEnd > selectionStart
      ? lineIndexAt(text, Math.max(0, selectionEnd - 1))
      : startLine;

  const starts = lineStartOffsets(text);
  let changed = false;
  let newStart = selectionStart;
  let newEnd = selectionEnd;

  for (let i = startLine; i <= endLine; i += 1) {
    const line = lines[i] ?? "";
    const lineStart = starts[i] ?? 0;

    if (outdent) {
      if (!line.startsWith(PARAGRAPH_INDENT)) continue;
      lines[i] = line.slice(INDENT_LEN);
      changed = true;
      if (selectionStart >= lineStart + INDENT_LEN) {
        newStart -= INDENT_LEN;
      } else if (selectionStart > lineStart) {
        newStart = lineStart;
      }
      if (selectionEnd >= lineStart + INDENT_LEN) {
        newEnd -= INDENT_LEN;
      } else if (selectionEnd > lineStart) {
        newEnd = lineStart;
      }
      continue;
    }

    if (line.startsWith(PARAGRAPH_INDENT)) continue;
    lines[i] = PARAGRAPH_INDENT + line;
    changed = true;
    if (selectionStart >= lineStart) newStart += INDENT_LEN;
    if (selectionEnd >= lineStart) newEnd += INDENT_LEN;
  }

  if (!changed) return null;
  return {
    text: lines.join("\n"),
    selectionStart: Math.max(0, newStart),
    selectionEnd: Math.max(0, newEnd),
  };
}

function isFenceMarker(line: string): boolean {
  const t = line.trim();
  return t.startsWith("```") || t.startsWith("~~~");
}

/** Markdown structure that should not receive novel-style first-line indent. */
function skipMarkdownStructure(
  line: string,
  inFence: boolean,
): { skip: boolean; inFence: boolean } {
  if (isFenceMarker(line)) return { skip: true, inFence: !inFence };
  if (inFence) return { skip: true, inFence: true };
  const t = line.trimStart();
  if (/^#{1,6}(?!#)/.test(t)) return { skip: true, inFence: false };
  if (/^>\s?/.test(t)) return { skip: true, inFence: false };
  if (/^([-*_])\1{2,}\s*$/.test(t)) return { skip: true, inFence: false };
  if (/^([-*+]|\d+\.)\s+/.test(t)) return { skip: true, inFence: false };
  if (/^\|/.test(t)) return { skip: true, inFence: false };
  if (/^ {4,}/.test(line) || line.startsWith("\t")) return { skip: true, inFence: false };
  return { skip: false, inFence: false };
}

/**
 * Toolbar action: first-line indent every prose paragraph in the file.
 * Skips blank lines; in Markdown also skips headings, lists, quotes, tables, and fences.
 */
export function applyDocumentFirstLineIndent(
  text: string,
  outdent: boolean,
  markdown: boolean,
  selectionStart = 0,
  selectionEnd = 0,
): { text: string; selectionStart: number; selectionEnd: number } | null {
  const lines = text.split("\n");
  if (lines.length === 0) return null;

  const starts = lineStartOffsets(text);
  let changed = false;
  let newStart = selectionStart;
  let newEnd = selectionEnd;
  let inFence = false;

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i] ?? "";
    const lineStart = starts[i] ?? 0;

    if (markdown) {
      const structure = skipMarkdownStructure(line, inFence);
      inFence = structure.inFence;
      if (structure.skip) continue;
    }
    if (!outdent && !line.trim()) continue;

    if (outdent) {
      if (!line.startsWith(PARAGRAPH_INDENT)) continue;
      lines[i] = line.slice(INDENT_LEN);
      changed = true;
      if (selectionStart >= lineStart + INDENT_LEN) {
        newStart -= INDENT_LEN;
      } else if (selectionStart > lineStart) {
        newStart = lineStart;
      }
      if (selectionEnd >= lineStart + INDENT_LEN) {
        newEnd -= INDENT_LEN;
      } else if (selectionEnd > lineStart) {
        newEnd = lineStart;
      }
      continue;
    }

    if (line.startsWith(PARAGRAPH_INDENT)) continue;
    lines[i] = PARAGRAPH_INDENT + line;
    changed = true;
    if (selectionStart >= lineStart) newStart += INDENT_LEN;
    if (selectionEnd >= lineStart) newEnd += INDENT_LEN;
  }

  if (!changed) return null;
  return {
    text: lines.join("\n"),
    selectionStart: Math.max(0, newStart),
    selectionEnd: Math.max(0, newEnd),
  };
}

export function buildDocumentFirstLineIndentChanges(
  text: string,
  outdent: boolean,
  markdown: boolean,
  selectionStart = 0,
  selectionEnd = 0,
): {
  changes: ParagraphIndentChange[];
  selectionStart: number;
  selectionEnd: number;
} | null {
  const result = applyDocumentFirstLineIndent(
    text,
    outdent,
    markdown,
    selectionStart,
    selectionEnd,
  );
  if (!result) return null;

  const oldLines = text.split("\n");
  const newLines = result.text.split("\n");
  const starts = lineStartOffsets(text);
  const changes: ParagraphIndentChange[] = [];

  for (let i = 0; i < oldLines.length; i += 1) {
    if (oldLines[i] === newLines[i]) continue;
    const lineStart = starts[i] ?? 0;
    if (outdent) {
      changes.push({ from: lineStart, to: lineStart + INDENT_LEN, insert: "" });
    } else {
      changes.push({ from: lineStart, to: lineStart, insert: PARAGRAPH_INDENT });
    }
  }

  if (changes.length === 0) return null;
  return {
    changes,
    selectionStart: result.selectionStart,
    selectionEnd: result.selectionEnd,
  };
}

/** Minimal per-line edits for CodeMirror (avoids full-doc replace + scroll jumps). */
export function buildParagraphIndentChanges(
  text: string,
  selectionStart: number,
  selectionEnd: number,
  outdent: boolean,
): {
  changes: ParagraphIndentChange[];
  selectionStart: number;
  selectionEnd: number;
} | null {
  const result = applyParagraphIndent(text, selectionStart, selectionEnd, outdent);
  if (!result) return null;

  const oldLines = text.split("\n");
  const newLines = result.text.split("\n");
  const startLine = lineIndexAt(text, selectionStart);
  const endLine =
    selectionEnd > selectionStart
      ? lineIndexAt(text, Math.max(0, selectionEnd - 1))
      : startLine;
  const starts = lineStartOffsets(text);
  const changes: ParagraphIndentChange[] = [];

  for (let i = startLine; i <= endLine; i += 1) {
    if (oldLines[i] === newLines[i]) continue;
    const lineStart = starts[i] ?? 0;
    if (outdent) {
      changes.push({ from: lineStart, to: lineStart + INDENT_LEN, insert: "" });
    } else {
      changes.push({ from: lineStart, to: lineStart, insert: PARAGRAPH_INDENT });
    }
  }

  if (changes.length === 0) return null;
  return {
    changes,
    selectionStart: result.selectionStart,
    selectionEnd: result.selectionEnd,
  };
}

export function handleReaderIndentKeyDown(
  e: KeyboardEvent<HTMLTextAreaElement>,
  text: string,
): { text: string; selectionStart: number; selectionEnd: number } | null {
  if (e.key !== "Tab" || e.nativeEvent.isComposing || e.keyCode === 229) return null;
  const ta = e.currentTarget;
  return applyParagraphIndent(text, ta.selectionStart, ta.selectionEnd, e.shiftKey);
}
