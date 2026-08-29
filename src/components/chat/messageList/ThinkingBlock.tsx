import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { highlightQuery } from "../../../utils/highlightQuery";
import { ChunkedText } from "./ChunkedText";
import { ThinkingChevronIcon, ThinkingIcon } from "./icons";
import { MIN_MEASURABLE_THINKING_MS, splitElapsed } from "./utils";

/** Matches the `.msg-thinking-panel` grid-template-rows transition (0.28s). */
const COLLAPSE_MS = 320;

/** Live counter cadence. Fast enough to look like a stopwatch, slow enough to
 *  stay off the render hot path while deltas are streaming. */
const TICK_MS = 100;

export function ThinkingBlock({
  content,
  streaming,
  startedAt,
  durationMs,
  highlightQuery: query,
}: {
  content: string;
  streaming: boolean;
  /** Epoch ms of the first reasoning delta, when known. */
  startedAt?: number;
  /** Span already measured for this block — authoritative once it stops. */
  durationMs?: number;
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

  // While reasoning streams the counter runs off the local clock rather than
  // `durationMs`, which only advances when a delta lands — a model that pauses
  // mid-thought would otherwise look like it had stopped the stopwatch.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!streaming || startedAt === undefined) return;
    setNow(Date.now());
    const timer = window.setInterval(() => setNow(Date.now()), TICK_MS);
    return () => window.clearInterval(timer);
  }, [streaming, startedAt]);

  const elapsed =
    streaming && startedAt !== undefined
      ? Math.max(durationMs ?? 0, now - startedAt)
      : (durationMs ?? 0);

  // Nothing to show for messages written before the span was recorded, or for
  // providers that deliver reasoning in a single chunk.
  const showElapsed = streaming
    ? startedAt !== undefined
    : elapsed >= MIN_MEASURABLE_THINKING_MS;
  const { minutes, seconds } = splitElapsed(elapsed);
  const elapsedLabel = streaming
    ? minutes > 0
      ? t("message.thinkingElapsedMinutes", { minutes, seconds })
      : t("message.thinkingElapsedSeconds", { seconds })
    : minutes > 0
      ? t("message.thinkingDoneMinutes", { minutes, seconds })
      : t("message.thinkingDoneSeconds", { seconds });

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
        {showElapsed && (
          // Tabular figures so the ticking counter does not shift the chevron.
          <span className="msg-thinking-elapsed">{elapsedLabel}</span>
        )}
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
