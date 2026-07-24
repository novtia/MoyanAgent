import { useCallback, useEffect, useRef, useState } from "react";
import type { PanelTab } from "../types";

export function useTabScroll(
  tabs: PanelTab[],
  activeTabId: string | null,
  width: number,
  open: boolean,
) {
  const tabsScrollRef = useRef<HTMLDivElement | null>(null);
  const [tabOverflow, setTabOverflow] = useState({ left: false, right: false });

  const updateTabOverflow = useCallback(() => {
    const el = tabsScrollRef.current;
    if (!el) return;
    const left = el.scrollLeft > 1;
    const right = el.scrollLeft + el.clientWidth < el.scrollWidth - 1;
    setTabOverflow((prev) =>
      prev.left === left && prev.right === right ? prev : { left, right },
    );
  }, []);

  const scrollTabs = useCallback((dir: -1 | 1) => {
    const el = tabsScrollRef.current;
    if (!el) return;
    el.scrollBy({ left: dir * Math.max(120, el.clientWidth * 0.7), behavior: "smooth" });
  }, []);

  // Keep overflow arrows accurate on tab count / panel width changes.
  useEffect(() => {
    const el = tabsScrollRef.current;
    if (!el) return;
    updateTabOverflow();
    const ro = new ResizeObserver(updateTabOverflow);
    ro.observe(el);
    return () => ro.disconnect();
  }, [updateTabOverflow, tabs.length, width, open]);

  // Translate vertical wheel into horizontal scroll (non-passive so we can
  // preventDefault and stop the wheel from bubbling to the page).
  useEffect(() => {
    const el = tabsScrollRef.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (el.scrollWidth <= el.clientWidth) return;
      if (e.deltaY === 0) return;
      e.preventDefault();
      el.scrollLeft += e.deltaY;
      updateTabOverflow();
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, [updateTabOverflow]);

  // Scroll the active tab into view when the selection changes.
  useEffect(() => {
    const el = tabsScrollRef.current;
    if (!el) return;
    const activeEl = el.querySelector<HTMLElement>(".right-panel-tab.is-active");
    if (activeEl) {
      const c = el.getBoundingClientRect();
      const a = activeEl.getBoundingClientRect();
      if (a.left < c.left) el.scrollLeft += a.left - c.left - 8;
      else if (a.right > c.right) el.scrollLeft += a.right - c.right + 8;
    }
    updateTabOverflow();
  }, [activeTabId, tabs, updateTabOverflow]);

  return {
    tabsScrollRef,
    tabOverflow,
    updateTabOverflow,
    scrollTabs,
  };
}
