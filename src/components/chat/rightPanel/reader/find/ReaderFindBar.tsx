import {
  useEffect,
  useMemo,
  useRef,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { normalizeReaderPath, useReader } from "../../../../../store/reader";
import {
  summarizeFindFiles,
  useReaderFind,
  type ReaderFindScope,
} from "../../../../../store/readerFind";
import type { ReaderFindBarProps } from "../types";
import { FindFileList } from "./FindFileList";
import { FindMatchList } from "./FindMatchList";
import { ChevronDownIcon, ChevronUpIcon, CloseIcon } from "./icons";

export type { ReaderFindBarProps } from "../types";
export { useReaderFindShortcuts } from "./useReaderFindShortcuts";

export function ReaderFindBar({ disabled, disabledReason }: ReaderFindBarProps) {
  const { t } = useTranslation();
  const open = useReaderFind((s) => s.open);
  const query = useReaderFind((s) => s.query);
  const replaceWith = useReaderFind((s) => s.replaceWith);
  const matchCase = useReaderFind((s) => s.matchCase);
  const scope = useReaderFind((s) => s.scope);
  const matchIndex = useReaderFind((s) => s.matchIndex);
  const matches = useReaderFind((s) => s.matches);
  const searching = useReaderFind((s) => s.searching);
  const setQuery = useReaderFind((s) => s.setQuery);
  const setReplaceWith = useReaderFind((s) => s.setReplaceWith);
  const setMatchCase = useReaderFind((s) => s.setMatchCase);
  const setScope = useReaderFind((s) => s.setScope);
  const close = useReaderFind((s) => s.close);
  const nextMatch = useReaderFind((s) => s.nextMatch);
  const prevMatch = useReaderFind((s) => s.prevMatch);
  const replaceCurrent = useReaderFind((s) => s.replaceCurrent);
  const replaceAll = useReaderFind((s) => s.replaceAll);
  const refreshMatches = useReaderFind((s) => s.refreshMatches);
  const goToFile = useReaderFind((s) => s.goToFile);
  const goToMatch = useReaderFind((s) => s.goToMatch);
  const activeTabId = useReader((s) => s.activeTabId);
  const readerTabs = useReader((s) => s.tabs);
  const findInputRef = useRef<HTMLInputElement>(null);
  const activeListBtnRef = useRef<HTMLButtonElement>(null);
  const isComposingRef = useRef(false);

  const fileSummaries = useMemo(
    () => (scope === "all" ? summarizeFindFiles(matches) : []),
    [scope, matches],
  );

  const hasQuery = query.trim().length > 0;
  const showResults = hasQuery && !searching;
  const showFileList = showResults && scope === "all";
  const showMatchList = showResults && scope === "file";
  const activeMatch = matchIndex >= 0 ? matches[matchIndex] ?? null : null;
  const matchCount = matches.length;
  const currentMatch = matchIndex >= 0 ? matchIndex + 1 : 0;

  const matchRows = useMemo(() => {
    if (!showMatchList) return [];
    const textByPath = new Map(
      readerTabs.map((tb) => [normalizeReaderPath(tb.path), tb.text] as const),
    );
    return matches.map((match, index) => {
      const text = textByPath.get(normalizeReaderPath(match.path)) ?? "";
      const lineStart = text.lastIndexOf("\n", Math.max(0, match.start - 1)) + 1;
      const lineEndIdx = text.indexOf("\n", match.start);
      const lineText = text.slice(lineStart, lineEndIdx < 0 ? text.length : lineEndIdx);
      const snippet = lineText.trim() || query;
      const localStart = Math.max(0, match.start - lineStart);
      const localEnd = Math.max(localStart, match.end - lineStart);
      return { match, index, snippet, localStart, localEnd };
    });
  }, [showMatchList, matches, readerTabs, query]);

  useEffect(() => {
    if ((!showFileList && !showMatchList) || matchIndex < 0) return;
    requestAnimationFrame(() => {
      activeListBtnRef.current?.scrollIntoView({ block: "nearest" });
    });
  }, [showFileList, showMatchList, matchIndex, matches]);

  useEffect(() => {
    if (open && scope === "file") {
      void refreshMatches();
    }
  }, [activeTabId, open, scope, refreshMatches]);

  useEffect(() => {
    if (open) {
      requestAnimationFrame(() => findInputRef.current?.focus());
    }
  }, [open]);

  if (!open) return null;

  const onFindKeyDown = (e: ReactKeyboardEvent) => {
    if (isComposingRef.current || e.nativeEvent.isComposing) return;
    if (e.key === "Enter") {
      e.preventDefault();
      if (e.shiftKey) prevMatch();
      else nextMatch();
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };

  const onReplaceKeyDown = (e: ReactKeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      void replaceCurrent();
    } else if (e.key === "Escape") {
      e.preventDefault();
      close();
    }
  };

  return (
    <div className="reader-find-bar" role="search" aria-label={t("readerFind.title")}>
      <div className="reader-find-head">
        <span className="reader-find-title">{t("readerFind.title")}</span>
        <button
          type="button"
          className="reader-find-close"
          title={t("readerFind.close")}
          onClick={() => close()}
        >
          <CloseIcon />
        </button>
      </div>

      <div className="reader-find-field">
        <label className="reader-find-label" htmlFor="reader-find-query">
          {t("readerFind.find")}
        </label>
        <div className="reader-find-search-row">
          <div className="reader-find-input-wrap">
            <input
              id="reader-find-query"
              ref={findInputRef}
              type="text"
              className="reader-find-input"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onCompositionStart={() => {
                isComposingRef.current = true;
              }}
              onCompositionEnd={(e) => {
                isComposingRef.current = false;
                setQuery(e.currentTarget.value);
              }}
              onKeyDown={onFindKeyDown}
              placeholder={t("readerFind.findPlaceholder")}
              disabled={disabled}
              autoComplete="off"
              spellCheck={false}
            />
            {hasQuery && (
              <span className="reader-find-count" aria-live="polite">
                {searching ? (
                  t("readerFind.searching")
                ) : matchCount > 0 ? (
                  `${currentMatch}/${matchCount}`
                ) : (
                  <span className="reader-find-count--empty">{t("readerFind.noResults")}</span>
                )}
              </span>
            )}
          </div>
          <div className="reader-find-nav" role="group">
            <button
              type="button"
              className="reader-find-nav-btn"
              title={t("readerFind.prev")}
              onClick={() => prevMatch()}
              disabled={disabled || matchCount === 0}
            >
              <ChevronUpIcon />
            </button>
            <button
              type="button"
              className="reader-find-nav-btn"
              title={t("readerFind.next")}
              onClick={() => nextMatch()}
              disabled={disabled || matchCount === 0}
            >
              <ChevronDownIcon />
            </button>
          </div>
        </div>
      </div>

      <div className="reader-find-field">
        <label className="reader-find-label" htmlFor="reader-find-replace">
          {t("readerFind.replace")}
        </label>
        <div className="reader-find-replace-row">
          <div className="reader-find-input-wrap reader-find-input-wrap--plain">
            <input
              id="reader-find-replace"
              type="text"
              className="reader-find-input"
              value={replaceWith}
              onChange={(e) => setReplaceWith(e.target.value)}
              onKeyDown={onReplaceKeyDown}
              placeholder={t("readerFind.replacePlaceholder")}
              disabled={disabled}
              autoComplete="off"
              spellCheck={false}
            />
          </div>
          <div className="reader-find-actions">
            <button
              type="button"
              className="reader-find-btn reader-find-btn--secondary"
              onClick={() => void replaceCurrent()}
              disabled={disabled || !hasQuery || matchCount === 0}
              title={t("readerFind.replaceOne")}
            >
              {t("readerFind.replaceOne")}
            </button>
            <button
              type="button"
              className="reader-find-btn reader-find-btn--primary"
              onClick={() => void replaceAll()}
              disabled={disabled || !hasQuery}
              title={t("readerFind.replaceAll")}
            >
              {t("readerFind.replaceAll")}
            </button>
          </div>
        </div>
      </div>

      <div className="reader-find-options">
        <div className="reader-find-scope" role="group" aria-label={t("readerFind.scope")}>
          {(["file", "all"] as ReaderFindScope[]).map((value) => (
            <button
              key={value}
              type="button"
              className={`reader-find-scope-btn${scope === value ? " is-active" : ""}`}
              onClick={() => setScope(value)}
              disabled={disabled}
            >
              {value === "file" ? t("readerFind.scopeFile") : t("readerFind.scopeAll")}
            </button>
          ))}
        </div>
        <label className="reader-find-check" title={t("readerFind.matchCase")}>
          <input
            type="checkbox"
            checked={matchCase}
            onChange={(e) => setMatchCase(e.target.checked)}
            disabled={disabled}
          />
          <span className="reader-find-check-box" aria-hidden />
          <span className="reader-find-check-text">{t("readerFind.matchCase")}</span>
        </label>
      </div>

      {showMatchList && (
        <FindMatchList
          matchRows={matchRows}
          matchCount={matchCount}
          matchIndex={matchIndex}
          query={query}
          disabled={disabled}
          activeListBtnRef={activeListBtnRef}
          onGoToMatch={goToMatch}
        />
      )}

      {showFileList && (
        <FindFileList
          fileSummaries={fileSummaries}
          activeMatch={activeMatch}
          disabled={disabled}
          activeListBtnRef={activeListBtnRef}
          onGoToFile={goToFile}
        />
      )}

      {disabled && disabledReason && (
        <p className="reader-find-disabled">{disabledReason}</p>
      )}
    </div>
  );
}
