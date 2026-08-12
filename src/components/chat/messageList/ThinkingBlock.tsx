import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { highlightQuery } from "../../../utils/highlightQuery";
import { ChunkedText } from "./ChunkedText";
import { ThinkingChevronIcon, ThinkingIcon } from "./icons";

/** Matches the `.msg-thinking-panel` grid-template-rows transition (0.28s). */
const COLLAPSE_MS = 320;

export function ThinkingBlock({
  content,
  streaming,
  highlightQuery: query,
}: {
  content: string;
  streaming: boolean;
  highlightQuery?: string;
}) {
  const { t } = useTranslation();
  const uid = useId();
  const panelId = `${uid}-thinking-panel`;
  const [open, setOpen] = useState(streaming);
  // Collapsed thinking text is dead weight — a long session accumulates a lot
  // of it — so it gets unmounted, but only once the collapse transition has
  // finished, otherwise the panel snaps shut instead of animating.
  const [mounted, setMounted] = useState(streaming);
  const userToggledRef = useRef(false);
  const prevStreamingRef = useRef(streaming);

  useEffect(() => {
    // Auto-collapse when streaming finishes, unless the user manually toggled.
    if (prevStreamingRef.current && !streaming && !userToggledRef.current) {
      setOpen(false);
    }
    prevStreamingRef.current = streaming;
  }, [streaming]);

  useEffect(() => {
    if (!query?.trim() || !content) return;
    if (content.toLocaleLowerCase().includes(query.trim().toLocaleLowerCase())) {
      setOpen(true);
    }
  }, [query, content]);

  useEffect(() => {
    if (open) {
      setMounted(true);
      return;
    }
    if (!mounted) return;
    const timer = window.setTimeout(() => setMounted(false), COLLAPSE_MS);
    return () => window.clearTimeout(timer);
  }, [open, mounted]);

  const handleToggle = () => {
    userToggledRef.current = true;
    setOpen((v) => !v);
  };

  return (
    <div
      className={`msg-thinking ${open ? "is-open" : ""} ${
        streaming ? "is-streaming" : ""
      }`}
    >
      <div
        className="msg-thinking-header"
        aria-expanded={open}
        aria-controls={panelId}
        title={t("message.thinkingHint")}
        onClick={handleToggle}
        role="button"
        tabIndex={0}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            handleToggle();
          }
        }}
      >
        <ThinkingIcon />
        <span className="msg-thinking-label">
          {streaming
            ? t("message.thinkingStreaming")
            : t("message.thinkingToggle")}
        </span>
        <ThinkingChevronIcon />
      </div>
      <div
        id={panelId}
        className="msg-thinking-panel"
        role="region"
        aria-hidden={!open}
      >
        <div className="msg-thinking-panel-inner">
          <div className="msg-thinking-content">
            {!mounted ? null : query ? (
              highlightQuery(content, query)
            ) : (
              <ChunkedText text={content} />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
