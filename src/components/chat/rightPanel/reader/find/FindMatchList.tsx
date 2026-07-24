import type { RefObject } from "react";
import { useTranslation } from "react-i18next";
import type { ReaderFindMatch } from "../../../../../store/readerFind";

export interface FindMatchRow {
  match: ReaderFindMatch;
  index: number;
  snippet: string;
  localStart: number;
  localEnd: number;
}

export function FindMatchList({
  matchRows,
  matchCount,
  matchIndex,
  query,
  disabled,
  activeListBtnRef,
  onGoToMatch,
}: {
  matchRows: FindMatchRow[];
  matchCount: number;
  matchIndex: number;
  query: string;
  disabled?: boolean;
  activeListBtnRef: RefObject<HTMLButtonElement | null>;
  onGoToMatch: (index: number) => void;
}) {
  const { t } = useTranslation();
  return (
    <div className="reader-find-file-panel">
      <div className="reader-find-file-panel-head">
        {t("readerFind.matchList")}
        {matchCount > 0 && (
          <span className="reader-find-file-panel-meta">
            {t("readerFind.totalMatches", { count: matchCount })}
          </span>
        )}
      </div>
      {matchRows.length === 0 ? (
        <p className="reader-find-file-empty">{t("readerFind.noResults")}</p>
      ) : (
        <ul
          className="reader-find-file-list reader-find-match-list"
          role="listbox"
          aria-label={t("readerFind.matchList")}
        >
          {matchRows.map((row) => {
            const isActive = matchIndex === row.index;
            const before = row.snippet.slice(0, row.localStart);
            const hit = row.snippet.slice(row.localStart, row.localEnd);
            const after = row.snippet.slice(row.localEnd);
            return (
              <li key={`${row.match.start}-${row.match.end}-${row.index}`} role="presentation">
                <button
                  ref={isActive ? (activeListBtnRef as React.RefObject<HTMLButtonElement>) : undefined}
                  type="button"
                  role="option"
                  aria-selected={isActive}
                  className={`reader-find-file-item reader-find-match-item${isActive ? " is-active" : ""}`}
                  title={t("readerFind.lineCol", {
                    line: row.match.line,
                    column: row.match.column,
                  })}
                  onClick={() => onGoToMatch(row.index)}
                  disabled={disabled}
                >
                  <span className="reader-find-match-line">
                    {t("readerFind.matchLine", { line: row.match.line })}
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
      )}
    </div>
  );
}
