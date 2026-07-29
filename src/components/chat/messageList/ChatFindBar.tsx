import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useChatFind } from "../../../store/chatFind";
import { highlightQuery } from "../../../utils/highlightQuery";

export function ChatFindBar() {
  const { t } = useTranslation();
  const open = useChatFind((s) => s.open);
  const query = useChatFind((s) => s.query);
  const hits = useChatFind((s) => s.hits);
  const hitIndex = useChatFind((s) => s.hitIndex);
  const loading = useChatFind((s) => s.loading);
  const listOpen = useChatFind((s) => s.listOpen);
  const nextMatch = useChatFind((s) => s.nextMatch);
  const prevMatch = useChatFind((s) => s.prevMatch);
  const goToHit = useChatFind((s) => s.goToHit);
  const close = useChatFind((s) => s.close);
  const setListOpen = useChatFind((s) => s.setListOpen);
  const activeBtnRef = useRef<HTMLButtonElement | null>(null);
  const [mounted, setMounted] = useState(false);

  useEffect(() => {
    setMounted(true);
  }, []);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null;
      const tag = target?.tagName?.toLowerCase();
      if (
        tag === "input" ||
        tag === "textarea" ||
        target?.isContentEditable
      ) {
        return;
      }
      if (event.key === "F3" || ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "g")) {
        event.preventDefault();
        if (event.shiftKey) prevMatch();
        else nextMatch();
      }
      if (event.key === "Escape") {
        event.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, nextMatch, prevMatch, close]);

  useEffect(() => {
    if (!listOpen || hitIndex < 0) return;
    requestAnimationFrame(() => {
      activeBtnRef.current?.scrollIntoView({ block: "nearest" });
    });
  }, [listOpen, hitIndex, hits]);

  if (!open || !mounted) return null;

  const total = hits.length;
  const current = hitIndex >= 0 ? hitIndex + 1 : 0;
  const countLabel = loading
    ? t("chatFind.searching")
    : total === 0
      ? t("chatFind.noResults")
      : t("chatFind.matchCount", { current, total });

  return (
    <div className="chat-find-dock">
      <div className="chat-find-bar" role="search" aria-label={t("chatFind.title")}>
      <div className="chat-find-bar-main">
        <span className="chat-find-query" title={query}>
          {query}
        </span>
        <span className="chat-find-count">{countLabel}</span>
        <div className="chat-find-actions">
          <button
            type="button"
            className="chat-find-btn"
            title={t("chatFind.prev")}
            aria-label={t("chatFind.prev")}
            disabled={total === 0}
            onClick={prevMatch}
          >
            <ChevronUpIcon />
          </button>
          <button
            type="button"
            className="chat-find-btn"
            title={t("chatFind.next")}
            aria-label={t("chatFind.next")}
            disabled={total === 0}
            onClick={nextMatch}
          >
            <ChevronDownIcon />
          </button>
          <button
            type="button"
            className={`chat-find-btn ${listOpen ? "is-active" : ""}`}
            title={t("chatFind.matchList")}
            aria-label={t("chatFind.matchList")}
            aria-pressed={listOpen}
            disabled={total === 0}
            onClick={() => setListOpen(!listOpen)}
          >
            <ListIcon />
          </button>
          <button
            type="button"
            className="chat-find-btn"
            title={t("chatFind.close")}
            aria-label={t("chatFind.close")}
            onClick={close}
          >
            <CloseIcon />
          </button>
        </div>
      </div>

      {listOpen && total > 0 && (
        <div className="chat-find-list" role="listbox">
          {hits.map((hit, index) => (
            <button
              key={`${hit.message_id}:${index}`}
              ref={index === hitIndex ? activeBtnRef : undefined}
              type="button"
              role="option"
              aria-selected={index === hitIndex}
              className={`chat-find-hit ${index === hitIndex ? "is-active" : ""}`}
              onClick={() => goToHit(index)}
            >
              <span className="chat-find-hit-snippet">
                {highlightQuery(hit.snippet || t("chatFind.emptySnippet"), query)}
              </span>
              {hit.match_fields.length > 0 && (
                <span className="chat-find-hit-tags">
                  {hit.match_fields.map((field) => (
                    <span key={field} className="chat-find-tag">
                      {t(`chatFind.field.${field}`, { defaultValue: field })}
                    </span>
                  ))}
                </span>
              )}
            </button>
          ))}
        </div>
      )}
      </div>
    </div>
  );
}

function ChevronUpIcon() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden>
      <path
        fill="currentColor"
        d="M8 5.2 3.6 9.6l.9.9L8 7l3.5 3.5.9-.9z"
      />
    </svg>
  );
}

function ChevronDownIcon() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden>
      <path
        fill="currentColor"
        d="M8 10.8 12.4 6.4l-.9-.9L8 9 4.5 5.5l-.9.9z"
      />
    </svg>
  );
}

function ListIcon() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden>
      <path
        fill="currentColor"
        d="M2 3h12v1.5H2zm0 4.25h12v1.5H2zm0 4.25h12V13H2z"
      />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden>
      <path
        fill="currentColor"
        d="M4.2 3.4 3.4 4.2 7.2 8l-3.8 3.8.8.8L8 8.8l3.8 3.8.8-.8L8.8 8l3.8-3.8-.8-.8L8 7.2z"
      />
    </svg>
  );
}
