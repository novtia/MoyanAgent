import { useEffect, useMemo, useRef, type KeyboardEvent as ReactKeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import { normalizeReaderPath, useReader } from "../../store/reader";
import {
  summarizeFindFiles,
  useReaderFind,
  type ReaderFindScope,
} from "../../store/readerFind";

function CloseIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden>
      <path d="M18 6L6 18M6 6l12 12" />
    </svg>
  );
}

interface ReaderFindBarProps {
  disabled?: boolean;
  disabledReason?: string;
}

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
  const activeTabId = useReader((s) => s.activeTabId);
  const findInputRef = useRef<HTMLInputElement>(null);
  const activeFileBtnRef = useRef<HTMLButtonElement>(null);

  const fileSummaries = useMemo(
    () => (scope === "all" ? summarizeFindFiles(matches) : []),
    [scope, matches],
  );

  const showFileList = scope === "all" && query.trim().length > 0 && !searching;
  const activeMatch = matchIndex >= 0 ? matches[matchIndex] ?? null : null;

  useEffect(() => {
    if (!showFileList || matchIndex < 0) return;
    requestAnimationFrame(() => {
      activeFileBtnRef.current?.scrollIntoView({ block: "nearest" });
    });
  }, [showFileList, matchIndex, matches]);

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

  const matchLabel =
    matches.length === 0
      ? t("readerFind.noResults")
      : matchIndex < 0
        ? t("readerFind.totalMatches", { count: matches.length })
        : t("readerFind.matchCount", {
            current: matchIndex + 1,
            total: matches.length,
          });

  const onFindKeyDown = (e: ReactKeyboardEvent) => {
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
      <div className="reader-find-row">
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
        <label className="reader-find-field">
          <span className="reader-find-label">{t("readerFind.find")}</span>
          <input
            ref={findInputRef}
            type="text"
            className="reader-find-input"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onFindKeyDown}
            placeholder={t("readerFind.findPlaceholder")}
            disabled={disabled}
          />
        </label>
        <label className="reader-find-check">
          <input
            type="checkbox"
            checked={matchCase}
            onChange={(e) => setMatchCase(e.target.checked)}
            disabled={disabled}
          />
          <span>{t("readerFind.matchCase")}</span>
        </label>
        <div className="reader-find-nav">
          <button
            type="button"
            className="reader-find-btn"
            title={t("readerFind.prev")}
            onClick={() => prevMatch()}
            disabled={disabled || matches.length === 0}
          >
            ↑
          </button>
          <button
            type="button"
            className="reader-find-btn"
            title={t("readerFind.next")}
            onClick={() => nextMatch()}
            disabled={disabled || matches.length === 0}
          >
            ↓
          </button>
        </div>
        <span className="reader-find-status">
          {searching ? t("readerFind.searching") : matchLabel}
        </span>
        <button
          type="button"
          className="reader-find-close"
          title={t("readerFind.close")}
          onClick={() => close()}
        >
          <CloseIcon />
        </button>
      </div>

      <div className="reader-find-row reader-find-row--replace">
        <label className="reader-find-field reader-find-field--replace">
          <span className="reader-find-label">{t("readerFind.replace")}</span>
          <input
            type="text"
            className="reader-find-input"
            value={replaceWith}
            onChange={(e) => setReplaceWith(e.target.value)}
            onKeyDown={onReplaceKeyDown}
            placeholder={t("readerFind.replacePlaceholder")}
            disabled={disabled}
          />
        </label>
        <button
          type="button"
          className="reader-find-action"
          onClick={() => void replaceCurrent()}
          disabled={disabled || !query || matches.length === 0}
        >
          {t("readerFind.replaceOne")}
        </button>
        <button
          type="button"
          className="reader-find-action"
          onClick={() => void replaceAll()}
          disabled={disabled || !query}
        >
          {t("readerFind.replaceAll")}
        </button>
      </div>

      {showFileList && (
        <div className="reader-find-file-panel">
          <div className="reader-find-file-panel-head">{t("readerFind.fileList")}</div>
          {fileSummaries.length === 0 ? (
            <p className="reader-find-file-empty">{t("readerFind.noResults")}</p>
          ) : (
            <ul className="reader-find-file-list" role="listbox" aria-label={t("readerFind.fileList")}>
              {fileSummaries.map((file) => {
                const isActive =
                  activeMatch != null &&
                  normalizeReaderPath(activeMatch.path) === normalizeReaderPath(file.path);
                return (
                  <li key={file.path} role="presentation">
                    <button
                      ref={isActive ? activeFileBtnRef : undefined}
                      type="button"
                      role="option"
                      aria-selected={isActive}
                      className={`reader-find-file-item${isActive ? " is-active" : ""}`}
                      title={file.path}
                      onClick={() => goToFile(file.path)}
                      disabled={disabled}
                    >
                      <span className="reader-find-file-name">{file.name}</span>
                      <span className="reader-find-file-count">
                        {t("readerFind.fileMatchCount", { count: file.count })}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      )}

      {disabled && disabledReason && (
        <p className="reader-find-disabled">{disabledReason}</p>
      )}
    </div>
  );
}

export function useReaderFindShortcuts(enabled: boolean) {
  const openFind = useReaderFind((s) => s.openFind);
  const close = useReaderFind((s) => s.close);
  const open = useReaderFind((s) => s.open);

  useEffect(() => {
    if (!enabled) return;
    const onKeyDown = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (!mod) return;
      const key = e.key.toLowerCase();
      if (key === "f" || key === "h") {
        e.preventDefault();
        openFind();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled, openFind]);

  useEffect(() => {
    if (!enabled || !open) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled, open, close]);
}
