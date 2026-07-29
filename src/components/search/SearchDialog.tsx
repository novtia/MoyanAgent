import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { useChatFind } from "../../store/chatFind";
import { useSession } from "../../store/session";
import { useSettings } from "../../store/settings";
import type { SessionSearchResult } from "../../types";
import { highlightQuery } from "../../utils/highlightQuery";

const RECENT_LIMIT = 5;
const SEARCH_LIMIT = 20;
const SEARCH_DEBOUNCE_MS = 120;

interface SearchDialogProps {
  open: boolean;
  onClose: () => void;
  onOpenChat: () => void;
}

interface SearchState {
  query: string;
  setQuery: (value: string) => void;
  results: SessionSearchResult[];
  loading: boolean;
  error: string | null;
}

function useSessionSearch(open: boolean): SearchState {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SessionSearchResult[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestIdRef = useRef(0);

  useEffect(() => {
    if (!open) {
      setQuery("");
      setResults([]);
      setLoading(false);
      setError(null);
      requestIdRef.current += 1;
      return;
    }

    const querySnapshot = query.trim();
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setLoading(true);
    setError(null);

    const timer = window.setTimeout(async () => {
      try {
        const next = await api.searchSessions(
          querySnapshot,
          querySnapshot ? SEARCH_LIMIT : RECENT_LIMIT,
        );
        if (requestIdRef.current !== requestId) return;
        setResults(next);
      } catch (err) {
        if (requestIdRef.current !== requestId) return;
        console.warn(err);
        setResults([]);
        setError(String(err));
      } finally {
        if (requestIdRef.current === requestId) {
          setLoading(false);
        }
      }
    }, querySnapshot ? SEARCH_DEBOUNCE_MS : 0);

    return () => window.clearTimeout(timer);
  }, [open, query]);

  return { query, setQuery, results, loading, error };
}

export function SearchDialog({ open, onClose, onOpenChat }: SearchDialogProps) {
  const { t } = useTranslation();
  const switchTo = useSession((s) => s.switchTo);
  const settings = useSettings((s) => s.settings);
  const { query, setQuery, results, loading, error } = useSessionSearch(open);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const isSearching = query.trim().length > 0;

  useEffect(() => {
    if (!open) return;
    setSelectedIndex(0);
    window.setTimeout(() => inputRef.current?.focus(), 0);
  }, [open]);

  useEffect(() => {
    setSelectedIndex(0);
  }, [query, results.length]);

  const activeModelLabel = useMemo(
    () => shortModelLabel(settings?.model || ""),
    [settings?.model],
  );

  const openResult = useCallback(
    async (result: SessionSearchResult | undefined) => {
      if (!result) return;
      const q = query.trim();
      await switchTo(result.id);
      onOpenChat();
      onClose();
      if (q) {
        void useChatFind.getState().activate({
          sessionId: result.id,
          query: q,
          preferredMessageId: result.match_message_id ?? undefined,
        });
      } else if (result.match_message_id) {
        window.setTimeout(() => {
          window.dispatchEvent(
            new CustomEvent("atelier:focus-message", {
              detail: { messageId: result.match_message_id },
            }),
          );
        }, 80);
      }
    },
    [onClose, onOpenChat, query, switchTo],
  );

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key === "ArrowDown") {
      event.preventDefault();
      setSelectedIndex((index) => Math.min(results.length - 1, index + 1));
      return;
    }
    if (event.key === "ArrowUp") {
      event.preventDefault();
      setSelectedIndex((index) => Math.max(0, index - 1));
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      openResult(results[selectedIndex]);
      return;
    }
    if ((event.ctrlKey || event.metaKey) && /^[1-9]$/.test(event.key)) {
      const shortcutIndex = Number(event.key) - 1;
      if (shortcutIndex < results.length) {
        event.preventDefault();
        openResult(results[shortcutIndex]);
      }
    }
  };

  if (!open) return null;

  return (
    <div
      className="search-backdrop"
      role="presentation"
      onMouseDown={onClose}
    >
      <div
        className="search-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={t("search.label")}
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <div className="search-input-wrap">
          <input
            ref={inputRef}
            className="search-input"
            type="search"
            value={query}
            placeholder={t("search.placeholder")}
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => setQuery(event.target.value)}
          />
        </div>

        <div className="search-section-title">
          {isSearching ? t("search.results") : t("search.recent")}
        </div>

        <div className="search-results" role="listbox">
          {results.map((result, index) => (
            <button
              key={result.id}
              type="button"
              className={`search-result ${selectedIndex === index ? "is-selected" : ""}`}
              role="option"
              aria-selected={selectedIndex === index}
              onMouseEnter={() => setSelectedIndex(index)}
              onClick={() => openResult(result)}
            >
              <span className="search-result-main">
                <span className="search-result-title">
                  {highlightQuery(result.title, query)}
                </span>
                {isSearching && (
                  <span className="search-result-snippet">
                    {highlightQuery(resultSummary(result, query, t), query)}
                  </span>
                )}
              </span>
              <span className="search-result-side">
                <span className="search-result-model">
                  {shortModelLabel(result.model || "") || activeModelLabel || "—"}
                </span>
                {index < RECENT_LIMIT && (
                  <kbd className="search-result-key">Ctrl+{index + 1}</kbd>
                )}
              </span>
            </button>
          ))}

          {!loading && results.length === 0 && (
            <div className="search-empty">
              {error ? t("search.error") : isSearching ? t("search.noResults") : t("search.noRecent")}
            </div>
          )}
          {loading && results.length === 0 && (
            <div className="search-empty">{t("search.loading")}</div>
          )}
        </div>
      </div>
    </div>
  );
}

function resultSummary(
  result: SessionSearchResult,
  query: string,
  t: (key: string, options?: Record<string, unknown>) => string,
) {
  if (result.match_text) return makeSnippet(result.match_text, query);
  if (result.title_match) return t("search.titleMatched");
  if (result.match_count > 0) {
    return t("search.messageMatches", { count: result.match_count });
  }
  return t("search.sessionMatched");
}

function shortModelLabel(model: string) {
  const trimmed = model.trim();
  if (!trimmed) return "";
  const parts = trimmed.split("/");
  return parts[parts.length - 1] || trimmed;
}

function makeSnippet(text: string, query: string) {
  const compact = text.replace(/\s+/g, " ").trim();
  const maxLength = 74;
  if (compact.length <= maxLength) return compact;

  const needle = query.trim().toLocaleLowerCase();
  const haystack = compact.toLocaleLowerCase();
  const index = needle ? haystack.indexOf(needle) : -1;
  const start = index >= 0 ? Math.max(0, index - 22) : 0;
  const end = Math.min(compact.length, start + maxLength);
  return `${start > 0 ? "..." : ""}${compact.slice(start, end)}${end < compact.length ? "..." : ""}`;
}
