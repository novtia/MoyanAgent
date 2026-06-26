import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../../store/session";
import type { MessageListProps } from "./types";
import { DevelopingRow } from "./DevelopingRow";
import { MessageRow } from "./MessageRow";

export function MessageList({ onPreviewImage }: MessageListProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const busy = useSession((s) => s.busy);
  const ref = useRef<HTMLDivElement | null>(null);
  const [focusedMessageId, setFocusedMessageId] = useState<string | null>(null);
  const messages = active?.messages || [];
  const lastMessageTextLength =
    messages.length > 0 ? messages[messages.length - 1].text?.length ?? 0 : 0;
  const lastMessageThinkingLength =
    messages.length > 0
      ? messages[messages.length - 1].params?.thinking_content?.length ?? 0
      : 0;
  const lastMessageBlocksLength =
    messages.length > 0
      ? messages[messages.length - 1].params?.blocks?.length ?? 0
      : 0;
  const hasStreamingAssistant = messages.some((m) => m.id.startsWith("tmp-assistant-"));

  // Track whether the user is near the bottom of the scroll container.
  // When they scroll up to read history we stop forcing scroll-to-bottom.
  const isNearBottomRef = useRef(true);
  const prevMessagesLengthRef = useRef(messages.length);

  // Listen for manual scrolls to update "near bottom" state.
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onScroll = () => {
      // Within 150 px of the bottom is considered "near bottom".
      isNearBottomRef.current =
        el.scrollHeight - el.scrollTop - el.clientHeight < 150;
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    return () => el.removeEventListener("scroll", onScroll);
  }, []);

  // Always scroll to bottom when switching sessions.
  useEffect(() => {
    if (!ref.current) return;
    ref.current.scrollTop = ref.current.scrollHeight;
    isNearBottomRef.current = true;
    prevMessagesLengthRef.current = messages.length;
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active?.session.id]);

  // Smart auto-scroll during streaming and when new messages arrive.
  useEffect(() => {
    if (!ref.current) return;
    const messagesGrew = messages.length > prevMessagesLengthRef.current;
    prevMessagesLengthRef.current = messages.length;
    // Always scroll when a new message is added (user just sent, or session
    // reloaded). During streaming only scroll when the user is already near
    // the bottom so we don't hijack their scroll position.
    if (messagesGrew || isNearBottomRef.current) {
      ref.current.scrollTop = ref.current.scrollHeight;
    }
  }, [
    messages.length,
    lastMessageTextLength,
    lastMessageThinkingLength,
    lastMessageBlocksLength,
    busy,
  ]);

  useEffect(() => {
    const onFocusMessage = (event: Event) => {
      const messageId = (event as CustomEvent<{ messageId?: string }>).detail?.messageId;
      if (!messageId) return;
      setFocusedMessageId(messageId);
      window.setTimeout(() => setFocusedMessageId((id) => (id === messageId ? null : id)), 1600);
    };
    window.addEventListener("atelier:focus-message", onFocusMessage);
    return () => window.removeEventListener("atelier:focus-message", onFocusMessage);
  }, []);

  useEffect(() => {
    if (!focusedMessageId || !ref.current) return;
    const selector = `[data-message-id="${focusedMessageId.replace(/["\\]/g, "\\$&")}"]`;
    const node = ref.current.querySelector<HTMLElement>(selector);
    node?.scrollIntoView({ block: "center", behavior: "smooth" });
  }, [focusedMessageId, active?.session.id, active?.messages.length]);

  const isEmpty = messages.length === 0 && !busy;

  return (
    <div className={`messages ${isEmpty ? "is-empty" : ""}`} ref={ref}>
      {isEmpty && (
        <div className="hero">
          <h1 className="hero-title">{t("chat.heroTitle")}</h1>
        </div>
      )}

      {!isEmpty && (
        <div className="messages-inner">
          {messages.map((m, index) => (
            <MessageRow
              key={`${m.id}:${index}`}
              m={m}
              onPreviewImage={onPreviewImage}
              focused={focusedMessageId === m.id}
            />
          ))}
          {busy && !hasStreamingAssistant && <DevelopingRow />}
        </div>
      )}
    </div>
  );
}
