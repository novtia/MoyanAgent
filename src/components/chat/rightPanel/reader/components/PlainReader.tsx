import { useMemo } from "react";
import {
  normalizeReaderPath,
  type ReaderFileTab,
} from "../../../../../store/reader";
import { useReaderFind } from "../../../../../store/readerFind";
import { findInText, resolveFindScrollIndex } from "../../../../../utils/readerFind";
import { ReaderCodeMirror } from "../ReaderCodeMirror";

export function PlainReader({
  tab,
  applyText,
}: {
  tab: ReaderFileTab;
  applyText: (text: string) => void;
}) {
  const findOpen = useReaderFind((s) => s.open);
  const findQuery = useReaderFind((s) => s.query);
  const matchCase = useReaderFind((s) => s.matchCase);
  const matchIndex = useReaderFind((s) => s.matchIndex);
  const findMatches = useReaderFind((s) => s.matches);

  const { ranges: findRanges, activeIndex: findActiveIndex } = useMemo(() => {
    if (!findOpen || !findQuery.trim() || tab.pendingDiffs.length > 0) {
      return { ranges: [], activeIndex: -1 };
    }
    const ranges = findInText(tab.text, findQuery, matchCase);
    if (matchIndex < 0) {
      return { ranges, activeIndex: -1 };
    }
    const activeMatch = findMatches[matchIndex] ?? null;
    if (
      !activeMatch ||
      normalizeReaderPath(activeMatch.path) !== normalizeReaderPath(tab.path)
    ) {
      return { ranges, activeIndex: -1 };
    }
    const activeIndex = ranges.findIndex(
      (r) => r.start === activeMatch.start && r.end === activeMatch.end,
    );
    if (activeIndex >= 0) {
      return { ranges, activeIndex };
    }
    const fileMatches = findMatches.filter(
      (m) => normalizeReaderPath(m.path) === normalizeReaderPath(tab.path),
    );
    const ord = fileMatches.findIndex(
      (m) => m.start === activeMatch.start && m.end === activeMatch.end,
    );
    return { ranges, activeIndex: ord >= 0 ? ord : -1 };
  }, [
    findOpen,
    findQuery,
    matchCase,
    tab.text,
    tab.path,
    tab.pendingDiffs.length,
    findMatches,
    matchIndex,
  ]);

  const showFindHighlight =
    findOpen && findQuery.trim().length > 0 && findRanges.length > 0;

  const scrollToIndex = useMemo(() => {
    if (!findOpen || matchIndex < 0 || tab.pendingDiffs.length > 0) return null;
    const activeMatch = findMatches[matchIndex] ?? null;
    if (
      !activeMatch ||
      normalizeReaderPath(activeMatch.path) !== normalizeReaderPath(tab.path)
    ) {
      return null;
    }
    const fileMatches = findMatches.filter(
      (m) => normalizeReaderPath(m.path) === normalizeReaderPath(tab.path),
    );
    return resolveFindScrollIndex(
      tab.text,
      findQuery,
      matchCase,
      activeMatch,
      fileMatches,
    );
  }, [
    findOpen,
    matchIndex,
    tab.path,
    tab.text,
    tab.pendingDiffs.length,
    findQuery,
    matchCase,
    findMatches,
  ]);

  return (
    <div className="reader-editor-wrap reader-editor-wrap--plain reader-editor-wrap--codemirror">
      <ReaderCodeMirror
        value={tab.text}
        onChange={applyText}
        ariaLabel={tab.path}
        findRanges={showFindHighlight ? findRanges : []}
        findActiveIndex={showFindHighlight ? findActiveIndex : -1}
        scrollToIndex={scrollToIndex}
        scrollTrigger={matchIndex}
      />
    </div>
  );
}
