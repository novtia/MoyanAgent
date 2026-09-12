import { useCallback, useEffect, useRef } from "react";
import { useRightPanel } from "../../../../store/rightPanel";
import type { TabKind } from "../types";

export function usePanelTabs(hasProjectPath: boolean) {
  const tabs = useRightPanel((s) => s.tabs);
  const setTabs = useRightPanel((s) => s.setTabs);
  const activeTabId = useRightPanel((s) => s.activeTabId);
  const setActiveTabId = useRightPanel((s) => s.setActiveTabId);
  const addTabStore = useRightPanel((s) => s.addTab);
  const setTabKindStore = useRightPanel((s) => s.setTabKind);
  const closeTab = useRightPanel((s) => s.closeTab);
  const closeOtherTabs = useRightPanel((s) => s.closeOtherTabs);
  const closeTabsToRight = useRightPanel((s) => s.closeTabsToRight);
  const closeAllTabs = useRightPanel((s) => s.closeAllTabs);

  const activeTabIdRef = useRef(activeTabId);
  useEffect(() => {
    activeTabIdRef.current = activeTabId;
  }, [activeTabId]);

  const addTab = useCallback(
    (kind: TabKind) => {
      if (kind === "reader" && !hasProjectPath) return;
      addTabStore(kind);
    },
    [addTabStore, hasProjectPath],
  );

  const setTabKind = useCallback(
    (id: string, kind: TabKind) => {
      if (kind === "reader" && !hasProjectPath) return;
      setTabKindStore(id, kind);
    },
    [hasProjectPath, setTabKindStore],
  );

  return {
    tabs,
    setTabs,
    activeTabId,
    setActiveTabId,
    activeTabIdRef,
    addTab,
    setTabKind,
    closeTab,
    closeOtherTabs,
    closeTabsToRight,
    closeAllTabs,
  };
}
