import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import { useSession } from "../../../store/session";
import type { MessageOutlineItem } from "../../../types";

const LEVEL_BASE = 10;
const LEVEL_STEP = 6;
const REPLY_PREVIEW_CHARS = 180;
/** Breathing room so the rail never touches the viewport edges. */
const RAIL_VERTICAL_INSET = 48;

function widthForLevel(level: number): number {
  return LEVEL_BASE + (level - 1) * LEVEL_STEP;
}

function focusMessage(messageId: string) {
  window.dispatchEvent(
    new CustomEvent("atelier:focus-message", {
      detail: { messageId },
    }),
  );
}

/** One tick = one full exchange: the user turn plus every reply it produced. */
interface TimelineTurn {
  /** Jump anchor — the user message when present, else the first message. */
  id: string;
  /** Every message belonging to this turn, for scroll-position mapping. */
  ids: string[];
  ask: string | null;
  reply: string | null;
}

function clip(text: string, limit: number): string {
  return text.length <= limit ? text : `${text.slice(0, limit - 1)}…`;
}

function buildTurns(outline: MessageOutlineItem[]): TimelineTurn[] {
  const turns: TimelineTurn[] = [];
  const replies: string[][] = [];
  let current: TimelineTurn | null = null;

  for (const item of outline) {
    if (item.id.startsWith("tmp-")) continue;
    if (item.role === "user" || !current) {
      current = { id: item.id, ids: [], ask: null, reply: null };
      turns.push(current);
      replies.push([]);
    }
    current.ids.push(item.id);
    const preview = item.preview?.trim() ?? "";
    if (!preview) continue;
    if (item.role === "user") {
      if (!current.ask) current.ask = preview;
    } else if (item.role === "assistant" || item.role === "error") {
      replies[replies.length - 1].push(preview);
    }
  }

  turns.forEach((turn, i) => {
    const joined = replies[i].join(" ").trim();
    turn.reply = joined ? clip(joined, REPLY_PREVIEW_CHARS) : null;
  });
  return turns;
}

interface RailBox {
  left: number;
  centerY: number;
  maxHeight: number;
}

interface MessageTimelineProps {
  /** The scroll container to align against (the message viewport). */
  scrollRef: RefObject<HTMLElement | null>;
  /** Message the reader is currently on — its turn's tick renders solid. */
  activeMessageId: string | null;
  onHoverChange?: (messageId: string | null) => void;
}

/**
 * Track the message viewport's screen box.
 *
 * The rail renders fixed in the body layer, so it needs the viewport's real
 * rect rather than any ancestor's layout: that is what makes it immune to the
 * sidebar collapsing, the right panel opening, or the message column resizing.
 */
function useRailBox(scrollRef: RefObject<HTMLElement | null>): RailBox | null {
  const [box, setBox] = useState<RailBox | null>(null);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const measure = () => {
      const r = el.getBoundingClientRect();
      const next: RailBox = {
        left: r.left,
        centerY: r.top + r.height / 2,
        maxHeight: Math.max(0, r.height - RAIL_VERTICAL_INSET),
      };
      setBox((prev) =>
        prev &&
        Math.abs(prev.left - next.left) < 0.5 &&
        Math.abs(prev.centerY - next.centerY) < 0.5 &&
        Math.abs(prev.maxHeight - next.maxHeight) < 0.5
          ? prev
          : next,
      );
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    window.addEventListener("resize", measure);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, [scrollRef]);

  return box;
}

/**
 * Left-rail ruler pinned to the message viewport: equal ticks at rest; hover
 * expands a 5→1 staircase with the peak tick in solid black
 * (see design/message-timeline.html).
 */
export function MessageTimeline({
  scrollRef,
  activeMessageId,
  onHoverChange,
}: MessageTimelineProps) {
  const { t } = useTranslation();
  const outline = useSession((s) => s.outline);
  const rail = useRailBox(scrollRef);

  const turns = useMemo(() => buildTurns(outline), [outline]);

  /** Any message id → the tick it belongs to. */
  const turnIndexById = useMemo(() => {
    const map = new Map<string, number>();
    turns.forEach((turn, i) => turn.ids.forEach((id) => map.set(id, i)));
    return map;
  }, [turns]);

  const trackRef = useRef<HTMLDivElement | null>(null);
  const [hoverIndex, setHoverIndex] = useState(-1);
  const hoverIndexRef = useRef(-1);
  const [tooltip, setTooltip] = useState<{
    ask: string;
    reply: string | null;
    top: number;
    left: number;
  } | null>(null);

  useEffect(() => {
    hoverIndexRef.current = hoverIndex;
  }, [hoverIndex]);

  const activeIndex =
    activeMessageId != null ? (turnIndexById.get(activeMessageId) ?? -1) : -1;

  const askOf = (turn: TimelineTurn) => turn.ask ?? t("chat.timelineAttachment");
  const replyOf = (turn: TimelineTurn) =>
    turn.reply ?? (turn.ids.length > 1 ? t("chat.timelineTools") : null);

  const updateTooltipForIndex = (idx: number) => {
    const turn = turns[idx];
    const nodes = trackRef.current?.querySelectorAll<HTMLElement>(".tl-tick");
    const node = nodes?.[idx];
    if (!turn || !node) {
      setTooltip(null);
      return;
    }
    const r = node.getBoundingClientRect();
    setTooltip({
      ask: askOf(turn),
      reply: replyOf(turn),
      top: r.top + r.height / 2,
      left: r.right + 10,
    });
  };

  useLayoutEffect(() => {
    if (hoverIndex < 0) {
      setTooltip(null);
      return;
    }
    updateTooltipForIndex(hoverIndex);
    // Reposition if layout shifts while hovering (width animation).
    const id = window.requestAnimationFrame(() =>
      updateTooltipForIndex(hoverIndex),
    );
    return () => window.cancelAnimationFrame(id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hoverIndex, turns]);

  if (turns.length === 0 || !rail) return null;

  const nearestIndex = (clientY: number) => {
    const nodes = trackRef.current?.querySelectorAll<HTMLElement>(".tl-tick");
    if (!nodes || nodes.length === 0) return 0;
    let best = 0;
    let bestDist = Infinity;
    nodes.forEach((node, i) => {
      const r = node.getBoundingClientRect();
      const cy = r.top + r.height / 2;
      const d = Math.abs(cy - clientY);
      if (d < bestDist) {
        bestDist = d;
        best = i;
      }
    });
    return best;
  };

  const levelOf = (i: number) =>
    hoverIndex >= 0 ? Math.max(1, 5 - Math.abs(i - hoverIndex)) : 1;

  const onPointerMove = (e: ReactPointerEvent) => {
    const idx = nearestIndex(e.clientY);
    if (idx !== hoverIndexRef.current) {
      setHoverIndex(idx);
      onHoverChange?.(turns[idx]?.id ?? null);
    } else {
      // Peak width may still be animating — keep tooltip glued to tick.
      updateTooltipForIndex(idx);
    }
  };

  const onPointerLeave = () => {
    setHoverIndex(-1);
    setTooltip(null);
    onHoverChange?.(null);
  };

  return createPortal(
    <>
      <aside
        className="message-timeline"
        aria-label={t("chat.timelineLabel")}
        style={{ left: rail.left, top: rail.centerY, height: rail.maxHeight }}
        onPointerMove={onPointerMove}
        onPointerLeave={onPointerLeave}
      >
        <div className="message-timeline-track" ref={trackRef}>
          {turns.map((turn, i) => {
            const level = levelOf(i);
            const isPeak = hoverIndex >= 0 && level === 5;
            // Scroll position only drives colour while the rail is idle.
            const isActive = hoverIndex < 0 && i === activeIndex;
            const reply = replyOf(turn);
            return (
              <button
                key={turn.id}
                type="button"
                className={`tl-tick${isPeak ? " is-peak" : ""}`}
                data-level={level}
                data-tl-message-id={turn.id}
                data-active={isActive ? "1" : "0"}
                aria-label={reply ? `${askOf(turn)} — ${reply}` : askOf(turn)}
                style={{ width: widthForLevel(level) }}
                onClick={() => focusMessage(turn.id)}
              />
            );
          })}
        </div>
      </aside>
      {tooltip && (
        <div
          className="tl-tooltip-fixed"
          style={{ top: tooltip.top, left: tooltip.left }}
          role="tooltip"
        >
          <p className="tl-tip-ask">{tooltip.ask}</p>
          {tooltip.reply && <p className="tl-tip-reply">{tooltip.reply}</p>}
        </div>
      )}
    </>,
    document.body,
  );
}
