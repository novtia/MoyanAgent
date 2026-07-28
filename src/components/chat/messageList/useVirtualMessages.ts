import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";

const DEFAULT_ROW = 120;
const OVERSCAN = 6;

export interface VirtualRange {
  start: number;
  end: number;
  offsetTop: number;
  totalHeight: number;
}

/**
 * Dynamic-height virtualization over a scroll parent.
 * Heights are measured via ResizeObserver after mount and stored in a map.
 */
export function useVirtualMessages(
  scrollRef: RefObject<HTMLElement | null>,
  count: number,
  opts?: { enabled?: boolean; estimate?: number },
) {
  const enabled = opts?.enabled ?? count >= 40;
  const estimate = opts?.estimate ?? DEFAULT_ROW;
  const heightsRef = useRef<Map<number, number>>(new Map());
  const [range, setRange] = useState<VirtualRange>({
    start: 0,
    end: Math.max(0, count),
    offsetTop: 0,
    totalHeight: count * estimate,
  });
  const [, bump] = useState(0);

  const totalHeight = useCallback(() => {
    let sum = 0;
    for (let i = 0; i < count; i++) {
      sum += heightsRef.current.get(i) ?? estimate;
    }
    return sum;
  }, [count, estimate]);

  const offsetAt = useCallback(
    (index: number) => {
      let sum = 0;
      for (let i = 0; i < index; i++) {
        sum += heightsRef.current.get(i) ?? estimate;
      }
      return sum;
    },
    [estimate],
  );

  const recompute = useCallback(() => {
    const el = scrollRef.current;
    if (!enabled || !el) {
      setRange({
        start: 0,
        end: count,
        offsetTop: 0,
        totalHeight: totalHeight(),
      });
      return;
    }
    const scrollTop = el.scrollTop;
    const viewH = el.clientHeight;
    let acc = 0;
    let start = 0;
    for (; start < count; start++) {
      const h = heightsRef.current.get(start) ?? estimate;
      if (acc + h > scrollTop) break;
      acc += h;
    }
    let end = start;
    let used = 0;
    for (; end < count; end++) {
      used += heightsRef.current.get(end) ?? estimate;
      if (used > viewH) break;
    }
    end = Math.min(count, end + 1 + OVERSCAN);
    start = Math.max(0, start - OVERSCAN);
    setRange({
      start,
      end,
      offsetTop: offsetAt(start),
      totalHeight: totalHeight(),
    });
  }, [count, enabled, estimate, offsetAt, scrollRef, totalHeight]);

  useLayoutEffect(() => {
    recompute();
  }, [recompute, count]);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !enabled) return;
    const onScroll = () => recompute();
    el.addEventListener("scroll", onScroll, { passive: true });
    const ro = new ResizeObserver(() => recompute());
    ro.observe(el);
    return () => {
      el.removeEventListener("scroll", onScroll);
      ro.disconnect();
    };
  }, [enabled, recompute, scrollRef]);

  const setRowHeight = useCallback(
    (index: number, height: number) => {
      if (height <= 0) return;
      const prev = heightsRef.current.get(index);
      if (prev != null && Math.abs(prev - height) < 1) return;
      heightsRef.current.set(index, height);
      bump((n) => n + 1);
      recompute();
    },
    [recompute],
  );

  const scrollToIndex = useCallback(
    (
      index: number,
      align: "start" | "center" | "end" = "center",
      behavior: ScrollBehavior = "auto",
    ) => {
      const el = scrollRef.current;
      if (!el) return;
      const clamped = Math.max(0, Math.min(count - 1, index));
      const top = offsetAt(clamped);
      const h = heightsRef.current.get(clamped) ?? estimate;
      let next = top;
      if (align === "center") next = top - el.clientHeight / 2 + h / 2;
      if (align === "end") next = top - el.clientHeight + h;
      el.scrollTo({ top: Math.max(0, next), behavior });
      // Force range recompute immediately for the new scrollTop.
      recompute();
    },
    [count, estimate, offsetAt, recompute, scrollRef],
  );

  const resetHeights = useCallback(() => {
    heightsRef.current.clear();
    recompute();
  }, [recompute]);

  return {
    enabled,
    range,
    setRowHeight,
    scrollToIndex,
    resetHeights,
    totalHeight: range.totalHeight,
  };
}
