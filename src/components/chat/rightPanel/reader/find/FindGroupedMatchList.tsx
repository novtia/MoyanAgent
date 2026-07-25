import type { RefObject } from "react";
import { useTranslation } from "react-i18next";
import { normalizeReaderPath } from "../../../../../store/reader";
import type {
  ReaderFindFileGroup,
  ReaderFindMatch,
} from "../../../../../store/readerFind";

export function FindGroupedMatchList({
  groups,
  matches,
  matchIndex,
  matchCount,
  query,
  disabled,
  activeListBtnRef,
  onGoToFile,
  onGoToMatch,
}: {
  groups: ReaderFindFileGroup[];
  matches: ReaderFindMatch[];
  matchIndex: number;
  matchCount: number;
  query: string;
  disabled?: boolean;
  activeListBtnRef: RefObject<HTMLButtonElement | null>;
  onGoToFile: (path: string) => void;
  onGoToMatch: (index: number) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="reader-find-file-panel">
      <div className="reader-find-file-panel-head">
        {t("readerFind.matchList")}
        {matchCount > 0 && (
          <span className="reader-find-file-panel-meta">
            {t("readerFind.groupedMatchMeta", {
              files: groups.length,
              matches: matchCount,
            })}
          </span>
        )}
      </div>
      {groups.length === 0 ? (
        <p className="reader-find-file-empty">{t("readerFind.noResults")}</p>
      ) : (
        <ul
          className="reader-find-file-list reader-find-grouped-list"
          role="listbox"
          aria-label={t("readerFind.matchList")}
        >
          {groups.map((group) => {
            const groupActive =
              matchIndex >= 0 &&
              matches[matchIndex] != null &&
              normalizeReaderPath(matches[matchIndex]!.path) ===
                normalizeReaderPath(group.path);
            return (
              <li key={group.path} className="reader-find-file-group" role="presentation">
                <button
                  type="button"
                  className={`reader-find-file-group-head${groupActive ? " is-active" : ""}`}
                  title={group.path}
                  onClick={() => onGoToFile(group.path)}
                  disabled={disabled}
                >
                  <span className="reader-find-file-name">{group.name}</span>
                  <span className="reader-find-file-count">
                    {t("readerFind.fileMatchCount", { count: group.count })}
                  </span>
                </button>
                <ul className="reader-find-file-group-matches" role="group">
                  {group.matchIndexes.map((index) => {
                    const match = matches[index];
                    if (!match) return null;
                    const isActive = matchIndex === index;
                    const before = match.snippet.slice(0, match.snippetStart);
                    const hit = match.snippet.slice(match.snippetStart, match.snippetEnd);
                    const after = match.snippet.slice(match.snippetEnd);
                    return (
                      <li key={`${group.path}:${index}`} role="presentation">
                        <button
                          ref={
                            isActive
                              ? (activeListBtnRef as RefObject<HTMLButtonElement>)
                              : undefined
                          }
                          type="button"
                          role="option"
                          aria-selected={isActive}
                          className={`reader-find-file-item reader-find-match-item${isActive ? " is-active" : ""}`}
                          title={`${group.name} · ${t("readerFind.lineCol", {
                            line: match.line,
                            column: match.column,
                          })}`}
                          onClick={() => onGoToMatch(index)}
                          disabled={disabled}
                        >
                          <span className="reader-find-match-line">
                            {t("readerFind.matchLine", { line: match.line })}
                          </span>
                          <span className="reader-find-match-snippet">
                            {before}
                            <mark className="reader-find-match-hit">{hit || query}</mark>
                            {after}
                          </span>
                        </button>
                      </li>
                    );
                  })}
                </ul>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
