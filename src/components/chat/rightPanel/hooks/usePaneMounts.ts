import { useEffect, useMemo, useState } from "react";
import { MAX_MOUNTED_PANES } from "../constants";
import type { PanelTab } from "../types";

function sameOrder(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((id, i) => id === b[i]);
}

/**
 * Which tab panes are kept mounted (keep-alive), most-recently-used first.
 *
 * A pane mounts the first time its tab is activated and then stays alive while
 * hidden, so scroll offsets, editors and selections survive tab switches. Past
 * {@link MAX_MOUNTED_PANES} the least recently used pane is dropped; its view
 * state is persisted on the tab, so re-activating restores it.
 */
export function usePaneMounts(tabs: PanelTab[], activeTabId: string | null): Set<string> {
  const [mounted, setMounted] = useState<string[]>([]);

  useEffect(() => {
    setMounted((prev) => {
      const live = new Set(tabs.map((tb) => tb.id));
      let next = prev.filter((id) => live.has(id));
      if (activeTabId && live.has(activeTabId)) {
        next = [activeTabId, ...next.filter((id) => id !== activeTabId)];
      }
      if (next.length > MAX_MOUNTED_PANES) next = next.slice(0, MAX_MOUNTED_PANES);
      return sameOrder(prev, next) ? prev : next;
    });
  }, [tabs, activeTabId]);

  return useMemo(() => new Set(mounted), [mounted]);
}
