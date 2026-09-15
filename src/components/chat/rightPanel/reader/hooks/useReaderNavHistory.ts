import { useCallback, useEffect, useRef } from "react";
import { normalizeReaderPath } from "../../../../../store/reader";
import { useRightPanel } from "../../../../../store/rightPanel";
import { usePanelTab } from "../../context/PanelTabContext";
import type { TabNavView } from "../../types";

const EMPTY: TabNavView = { stack: [], index: -1 };

/**
 * Back/forward over the files visited in one reader tab. The stack lives in
 * the tab's view state (rewritten centrally when files are renamed or
 * deleted), so each tab navigates its own trail.
 */
export function useReaderNavHistory(path: string | null | undefined) {
  const { tabId } = usePanelTab();
  const history = useRightPanel(
    (s) => s.tabs.find((tb) => tb.id === tabId)?.view?.history ?? EMPTY,
  );
  const updateTabView = useRightPanel((s) => s.updateTabView);
  const setTabPath = useRightPanel((s) => s.setTabPath);
  /** Set while we drive the path ourselves, so the trail is not re-appended. */
  const navPendingRef = useRef(false);

  useEffect(() => {
    if (!path) return;
    if (navPendingRef.current) {
      navPendingRef.current = false;
      return;
    }
    const current = history.index >= 0 ? history.stack[history.index] : null;
    if (current && normalizeReaderPath(current) === normalizeReaderPath(path)) return;
    const stack = [...history.stack.slice(0, history.index + 1), path];
    updateTabView(tabId, { history: { stack, index: stack.length - 1 } });
  }, [path, history, tabId, updateTabView]);

  const go = useCallback(
    (index: number) => {
      const target = history.stack[index];
      if (!target) return;
      navPendingRef.current = true;
      updateTabView(tabId, { history: { stack: history.stack, index } });
      setTabPath(tabId, target);
    },
    [history, tabId, updateTabView, setTabPath],
  );

  return {
    canBack: history.index > 0,
    canForward: history.index >= 0 && history.index < history.stack.length - 1,
    goBack: useCallback(() => {
      if (history.index <= 0) return;
      go(history.index - 1);
    }, [history.index, go]),
    goForward: useCallback(() => {
      if (history.index >= history.stack.length - 1) return;
      go(history.index + 1);
    }, [history.index, history.stack.length, go]),
  };
}
