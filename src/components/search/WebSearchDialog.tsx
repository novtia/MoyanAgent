import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { useSession } from "../../store/session";
import { copyText } from "../../utils/clipboard";
import type { WebSearchHit } from "../../types";

interface WebSearchDialogProps {
  open: boolean;
  onClose: () => void;
}

export function WebSearchDialog({ open, onClose }: WebSearchDialogProps) {
  const { t } = useTranslation();
  const setPrompt = useSession((s) => s.setPrompt);
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<WebSearchHit[]>([]);
  const [backend, setBackend] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const requestIdRef = useRef(0);

  useEffect(() => {
    if (!open) {
      setQuery("");
      setHits([]);
      setBackend("");
      setLoading(false);
      setError(null);
      requestIdRef.current += 1;
      return;
    }
    window.setTimeout(() => inputRef.current?.focus(), 0);
  }, [open]);

  const runSearch = async () => {
    const q = query.trim();
    if (!q) return;
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setLoading(true);
    setError(null);
    try {
      const outcome = await api.webSearch(q);
      if (requestIdRef.current !== requestId) return;
      setHits(outcome.hits);
      setBackend(outcome.backend);
    } catch (err) {
      if (requestIdRef.current !== requestId) return;
      setHits([]);
      setError(String(err));
    } finally {
      if (requestIdRef.current === requestId) setLoading(false);
    }
  };

  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      void runSearch();
    }
  };

  const insertIntoComposer = (hit: WebSearchHit) => {
    const snippet = `${hit.title} - ${hit.url}`;
    const current = useSession.getState().composer.prompt;
    setPrompt(current ? `${current}\n${snippet}` : snippet);
    onClose();
  };

  if (!open) return null;

  return (
    <div className="search-backdrop" role="presentation" onMouseDown={onClose}>
      <div
        className="search-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={t("settings.search.title")}
        onMouseDown={(event) => event.stopPropagation()}
        onKeyDown={onKeyDown}
      >
        <div className="search-input-wrap" style={{ display: "flex", gap: 8 }}>
          <input
            ref={inputRef}
            className="search-input"
            type="search"
            value={query}
            placeholder={t("settings.search.inputPlaceholder")}
            autoComplete="off"
            spellCheck={false}
            style={{ flex: 1 }}
            onChange={(event) => setQuery(event.target.value)}
          />
          <button
            type="button"
            className="btn"
            disabled={loading || query.trim().length === 0}
            onClick={() => void runSearch()}
          >
            {loading ? t("settings.search.searching") : t("settings.search.searchAction")}
          </button>
        </div>

        {backend && (
          <div className="search-section-title">
            {t("settings.search.backendTitle")}: {backend}
          </div>
        )}

        <div className="search-results" role="list">
          {hits.map((hit, index) => (
            <div key={`${hit.url}:${index}`} className="search-result" role="listitem">
              <span className="search-result-main">
                <button
                  type="button"
                  className="search-result-title"
                  style={{
                    textAlign: "left",
                    background: "none",
                    border: "none",
                    cursor: "pointer",
                    padding: 0,
                    color: "inherit",
                  }}
                  title={t("settings.search.openLink")}
                  onClick={() => api.openUrl(hit.url).catch(console.warn)}
                >
                  {hit.title || hit.url}
                </button>
                {hit.snippet && (
                  <span className="search-result-snippet">{hit.snippet}</span>
                )}
                <span
                  className="search-result-snippet"
                  style={{ opacity: 0.6, fontSize: "0.78em" }}
                >
                  {hit.url}
                </span>
              </span>
              <span
                className="search-result-side"
                style={{ display: "flex", gap: 6 }}
              >
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => insertIntoComposer(hit)}
                >
                  {t("settings.search.insert")}
                </button>
                <button
                  type="button"
                  className="btn btn-sm"
                  onClick={() => copyText(hit.url).catch(console.warn)}
                >
                  URL
                </button>
              </span>
            </div>
          ))}

          {!loading && hits.length === 0 && (
            <div className="search-empty">
              {error ? `${t("settings.search.error")}: ${error}` : t("settings.search.empty")}
            </div>
          )}
          {loading && hits.length === 0 && (
            <div className="search-empty">{t("settings.search.searching")}</div>
          )}
        </div>
      </div>
    </div>
  );
}
