import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../../store/session";
import type { MessageListProps } from "./types";
import { DevelopingRow } from "./DevelopingRow";
import { MessageRow } from "./MessageRow";
import { MessageTimeline } from "./MessageTimeline";
import { useVirtualMessages } from "./useVirtualMessages";

function VirtualRow({
  index,
  onHeight,
  tail,
  children,
}: {
  index: number;
  onHeight: (index: number, height: number) => void;
  tail?: boolean;
  children: React.ReactNode;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const publish = () => onHeight(index, el.getBoundingClientRect().height);
    publish();
    const ro = new ResizeObserver(publish);
    ro.observe(el);
    return () => ro.disconnect();
  }, [index, onHeight]);
  return (
    <div
      className="msg-virtual-row"
      ref={ref}
      data-msg-index={index}
      data-msg-tail={tail ? "1" : undefined}
    >
      {children}
    </div>
  );
}

function escapeAttrSelector(id: string): string {
  return id.replace(/["\\]/g, "\\$&");
}

export function MessageList({ onPreviewImage }: MessageListProps) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const busy = useSession((s) => s.busy);
  const outline = useSession((s) => s.outline);
  const loadOlderMessages = useSession((s) => s.loadOlderMessages);
  const ensureMessageLoaded = useSession((s) => s.ensureMessageLoaded);
  const messagesWindowHasMoreBefore = useSession(
    (s) => s.messagesWindowHasMoreBefore,
  );
  const messagesLoading = useSession((s) => s.messagesLoading);

  const ref = useRef<HTMLDivElement | null>(null);
  const [focusedMessageId, setFocusedMessageId] = useState<string | null>(null);
  const [scrollActiveId, setScrollActiveId] = useState<string | null>(null);
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
  const hasStreamingAssistant = messages.some((m) =>
    m.id.startsWith("tmp-assistant-"),
  );

  const isNearBottomRef = useRef(true);
  const suppressAutoScrollRef = useRef(false);
  const prevMessagesLengthRef = useRef(messages.length);
  const prefetchLock = useRef(false);
  const focusTokenRef = useRef(0);
  const jumpingRef = useRef(false);

  const {
    enabled: virtualEnabled,
    range,
    setRowHeight,
    scrollToIndex,
    resetHeights,
  } = useVirtualMessages(ref, messages.length);

  const firstMessageId = messages[0]?.id ?? "";

  useEffect(() => {
    resetHeights();
  }, [active?.session.id, firstMessageId, resetHeights]);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onScroll = () => {
      if (!suppressAutoScrollRef.current) {
        isNearBottomRef.current =
          el.scrollHeight - el.scrollTop - el.clientHeight < 150;
      }

      // Prefetch older history near the top of the loaded window.
      // Never while a timeline jump is converging — it would shift the anchor.
      if (
        messagesWindowHasMoreBefore &&
        !messagesLoading &&
        !prefetchLock.current &&
        !jumpingRef.current &&
        el.scrollTop < 280
      ) {
        prefetchLock.current = true;
        const prevHeight = el.scrollHeight;
        const prevTop = el.scrollTop;
        void loadOlderMessages().finally(() => {
          requestAnimationFrame(() => {
            if (ref.current) {
              const delta = ref.current.scrollHeight - prevHeight;
              ref.current.scrollTop = prevTop + delta;
            }
            prefetchLock.current = false;
          });
        });
      }

      // Reading position for the timeline: last message that starts above the
      // probe line, so a tall message keeps its tick lit while scrolling it.
      const probe = el.getBoundingClientRect().top + el.clientHeight * 0.35;
      const nodes = el.querySelectorAll<HTMLElement>(".msg[data-message-id]");
      let activeId: string | null = nodes[0]?.dataset.messageId ?? null;
      nodes.forEach((node) => {
        if (node.getBoundingClientRect().top <= probe) {
          activeId = node.dataset.messageId ?? activeId;
        }
      });
      if (activeId) setScrollActiveId(activeId);
    };
    el.addEventListener("scroll", onScroll, { passive: true });
    // Seed the reading position without waiting for the first scroll event.
    const seed = requestAnimationFrame(onScroll);
    return () => {
      cancelAnimationFrame(seed);
      el.removeEventListener("scroll", onScroll);
    };
  }, [
    loadOlderMessages,
    messagesLoading,
    messagesWindowHasMoreBefore,
    messages.length,
  ]);

  useEffect(() => {
    if (!ref.current) return;
    ref.current.scrollTop = ref.current.scrollHeight;
    isNearBottomRef.current = true;
    suppressAutoScrollRef.current = false;
    prevMessagesLengthRef.current = messages.length;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active?.session.id]);

  useEffect(() => {
    if (!ref.current) return;
    if (suppressAutoScrollRef.current) {
      prevMessagesLengthRef.current = messages.length;
      return;
    }
    const messagesGrew = messages.length > prevMessagesLengthRef.current;
    prevMessagesLengthRef.current = messages.length;
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
    const findNode = (messageId: string) => {
      const el = ref.current;
      if (!el) return null;
      // Must be scoped to `.msg`: timeline ticks live inside the scroller and
      // also carry data-message-id.
      return el.querySelector<HTMLElement>(
        `.msg[data-message-id="${escapeAttrSelector(messageId)}"]`,
      );
    };

    const nextFrame = () =>
      new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));

    /** Index span of the rows the virtual window currently has mounted. */
    const mountedIndexRange = () => {
      const el = ref.current;
      if (!el) return null;
      let min = Infinity;
      let max = -Infinity;
      el.querySelectorAll<HTMLElement>(
        "[data-msg-index]:not([data-msg-tail])",
      ).forEach((node) => {
        const i = Number(node.dataset.msgIndex);
        if (Number.isNaN(i)) return;
        if (i < min) min = i;
        if (i > max) max = i;
      });
      return min === Infinity ? null : { min, max };
    };

    /**
     * Converge on the target row with instant scroll steps.
     *
     * Smooth scrolling cannot be used here: measuring rows re-renders the
     * virtual window, which cancels an in-flight smooth scroll after a few
     * pixels. Row offsets are also only estimates until measured, so a single
     * `scrollToIndex` can land far off. Instead we binary-search `scrollTop`
     * using the mounted index span as the comparator, then centre the row from
     * its real measured rect.
     */
    const scrollMessageIntoView = async (
      messageId: string,
      index: number,
      token: number,
    ) => {
      const el = ref.current;
      if (!el) return;

      // Estimated jump first — often already lands in the right band.
      if (virtualEnabled && index >= 0) {
        scrollToIndex(index, "center", "auto");
        await nextFrame();
      }

      let lo = 0;
      let hi = Math.max(0, el.scrollHeight - el.clientHeight);

      for (let attempt = 0; attempt < 48; attempt++) {
        if (focusTokenRef.current !== token) return;

        const node = findNode(messageId);
        if (node) {
          const containerRect = el.getBoundingClientRect();
          const nodeRect = node.getBoundingClientRect();
          const viewportCenter = containerRect.top + el.clientHeight / 2;
          const nodeCenter = nodeRect.top + nodeRect.height / 2;
          const delta = nodeCenter - viewportCenter;
          if (Math.abs(delta) <= 6) return;
          const before = el.scrollTop;
          el.scrollTop = Math.max(0, before + delta);
          // Clamped by scroll bounds: as close as this row can get.
          if (Math.abs(el.scrollTop - before) < 1) return;
        } else {
          const mounted = mountedIndexRange();
          if (!mounted) return;
          hi = Math.max(hi, el.scrollHeight - el.clientHeight);
          if (index < mounted.min) {
            hi = Math.max(lo, el.scrollTop - 1);
          } else if (index > mounted.max) {
            lo = Math.min(hi, el.scrollTop + 1);
          } else {
            // In range but not in the DOM yet — wait for the commit.
            await nextFrame();
            continue;
          }
          if (hi <= lo) return;
          el.scrollTop = Math.floor((lo + hi) / 2);
        }

        await nextFrame();
      }
    };

    const onFocusMessage = (event: Event) => {
      const messageId = (event as CustomEvent<{ messageId?: string }>).detail
        ?.messageId;
      if (!messageId) return;
      const token = ++focusTokenRef.current;
      void (async () => {
        suppressAutoScrollRef.current = true;
        jumpingRef.current = true;
        isNearBottomRef.current = false;

        try {
          const hadAlready = useSession
            .getState()
            .active?.messages.some((m) => m.id === messageId);
          const idx = await ensureMessageLoaded(messageId);
          if (focusTokenRef.current !== token) return;

          // Window was replaced — row indices remapped, so stale heights lie.
          if (!hadAlready) {
            resetHeights();
            await nextFrame();
          }

          setFocusedMessageId(messageId);
          window.setTimeout(
            () => setFocusedMessageId((id) => (id === messageId ? null : id)),
            1600,
          );

          if (idx < 0) return;
          await scrollMessageIntoView(messageId, idx, token);
        } finally {
          if (focusTokenRef.current === token) {
            jumpingRef.current = false;
            suppressAutoScrollRef.current = false;
          }
        }
      })();
    };
    window.addEventListener("atelier:focus-message", onFocusMessage);
    return () =>
      window.removeEventListener("atelier:focus-message", onFocusMessage);
  }, [ensureMessageLoaded, resetHeights, scrollToIndex, virtualEnabled]);

  const isEmpty = messages.length === 0 && !busy;
  const showTimeline = !isEmpty && outline.length > 0;

  const start = virtualEnabled ? range.start : 0;
  const end = virtualEnabled ? range.end : messages.length;
  const slice = messages.slice(start, end);

  const spacerStyle: CSSProperties | undefined = virtualEnabled
    ? {
        height: range.totalHeight,
        position: "relative",
      }
    : undefined;

  const innerStyle: CSSProperties | undefined = virtualEnabled
    ? {
        position: "absolute",
        top: range.offsetTop,
        left: 0,
        right: 0,
      }
    : undefined;

  const forceTail =
    virtualEnabled &&
    (hasStreamingAssistant || busy) &&
    end < messages.length;

  return (
    <div className={`messages ${isEmpty ? "is-empty" : ""}`} ref={ref}>
      {isEmpty && (
        <div className="hero">
          <h1 className="hero-title">{t("chat.heroTitle")}</h1>
        </div>
      )}

      {!isEmpty && (
        <div className="messages-shell">
          {showTimeline && (
            <MessageTimeline activeMessageId={scrollActiveId} />
          )}
          <div className="messages-inner">
            {messagesLoading && messagesWindowHasMoreBefore && (
              <div className="messages-loading-older" aria-hidden>
                …
              </div>
            )}
            <div className="messages-virtual" style={spacerStyle}>
              <div style={innerStyle}>
                {slice.map((m, i) => {
                  const index = start + i;
                  const row = (
                    <MessageRow
                      key={`${m.id}:${index}`}
                      m={m}
                      onPreviewImage={onPreviewImage}
                      focused={focusedMessageId === m.id}
                    />
                  );
                  if (!virtualEnabled) return row;
                  return (
                    <VirtualRow
                      key={`${m.id}:${index}`}
                      index={index}
                      onHeight={setRowHeight}
                    >
                      {row}
                    </VirtualRow>
                  );
                })}
                {forceTail &&
                  messages
                    .slice(Math.max(end, messages.length - 2))
                    .map((m, i) => {
                      const index = Math.max(end, messages.length - 2) + i;
                      if (index < end) return null;
                      return (
                        <VirtualRow
                          key={`tail-${m.id}:${index}`}
                          index={index}
                          onHeight={setRowHeight}
                          tail
                        >
                          <MessageRow
                            m={m}
                            onPreviewImage={onPreviewImage}
                            focused={focusedMessageId === m.id}
                          />
                        </VirtualRow>
                      );
                    })}
                {busy && !hasStreamingAssistant && <DevelopingRow />}
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
