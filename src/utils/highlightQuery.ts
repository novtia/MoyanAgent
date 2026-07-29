/**
 * Case-insensitive highlight of `query` within `text` as React nodes with <mark>.
 * Returns the original string when query is empty or not found.
 */
import { createElement, type ReactNode } from "react";

export function highlightQuery(text: string, query: string): ReactNode {
  const needle = query.trim();
  if (!needle || !text) return text;

  const lowerText = text.toLocaleLowerCase();
  const lowerNeedle = needle.toLocaleLowerCase();
  const parts: ReactNode[] = [];
  let cursor = 0;
  let key = 0;

  while (cursor < text.length) {
    const start = lowerText.indexOf(lowerNeedle, cursor);
    if (start < 0) {
      parts.push(text.slice(cursor));
      break;
    }
    if (start > cursor) {
      parts.push(text.slice(cursor, start));
    }
    const end = start + needle.length;
    parts.push(
      createElement("mark", { key: `m${key++}`, className: "chat-find-mark" }, text.slice(start, end)),
    );
    cursor = end;
  }

  if (parts.length === 1 && typeof parts[0] === "string") {
    return parts[0];
  }
  return parts;
}
