import { type ReactNode, type RefObject } from "react";
import type { TextRange } from "../../utils/readerFind";

export function buildHighlightedParts(
  text: string,
  ranges: TextRange[],
  activeIndex: number,
): ReactNode[] {
  if (ranges.length === 0) return [text];
  const parts: ReactNode[] = [];
  let last = 0;
  ranges.forEach((range, index) => {
    if (range.start > last) {
      parts.push(text.slice(last, range.start));
    }
    parts.push(
      <mark
        key={`${range.start}-${range.end}`}
        className={index === activeIndex && activeIndex >= 0 ? "is-active" : undefined}
      >
        {text.slice(range.start, range.end)}
      </mark>,
    );
    last = range.end;
  });
  if (last < text.length) {
    parts.push(text.slice(last));
  }
  return parts;
}

export function ReaderFindBackdrop({
  text,
  ranges,
  activeIndex,
  innerRef,
}: {
  text: string;
  ranges: TextRange[];
  activeIndex: number;
  innerRef: RefObject<HTMLDivElement | null>;
}) {
  return (
    <div className="reader-editor-find-backdrop" aria-hidden>
      <div ref={innerRef as RefObject<HTMLDivElement>} className="reader-editor-find-backdrop-inner">
        {buildHighlightedParts(text, ranges, activeIndex)}
      </div>
    </div>
  );
}
