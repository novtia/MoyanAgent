import { useCallback, useEffect, useRef, useState } from "react";
import {
  newTabId,
  persistTabs,
  pickActiveId,
  readStoredTabs,
} from "../storage/panelTabs";
import type { PanelTab, TabKind } from "../types";

export function usePanelTabs(
  activeSessionId: string | null,
  hasProjectPath: boolean,
) {
  const initial = useRef(readStoredTabs(activeSessionId));
  const [tabs, setTabs] = useState<PanelTab[]>(initial.current.tabs);
  const [activeTabId, setActiveTabId] = useState<string | null>(initial.current.activeId);
  /** Session whose `tabs` / `activeTabId` currently belong to. */
  const boundSessionRef = useRef<string | null>(activeSessionId);
  /** Skip one persist after a session swap so stale tabs are not written. */
  const skipPersistRef = useRef(false);

  const activeTabIdRef = useRef(activeTabId);
  useEffect(() => {
    activeTabIdRef.current = activeTabId;
  }, [activeTabId]);

  // Per-session panel tabs: save outgoing session, load incoming session.
  useEffect(() => {
    const prev = boundSessionRef.current;
    if (prev === activeSessionId) return;

    if (prev) {
      persistTabs(prev, tabs, activeTabId);
    }

    skipPersistRef.current = true;
    boundSessionRef.current = activeSessionId;
    if (!activeSessionId) {
      setTabs([]);
      setActiveTabId(null);
      return;
    }

    const loaded = readStoredTabs(activeSessionId);
    setTabs(loaded.tabs);
    setActiveTabId(pickActiveId(loaded.tabs, loaded.activeId));
    // Intentionally only react to session switches; tabs/activeTabId are the
    // outgoing session's values captured on that render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeSessionId]);

  // Sessions without a project path cannot use the document reader.
  useEffect(() => {
    if (hasProjectPath) return;
    setTabs((prev) => {
      const next = prev.filter((tb) => tb.kind !== "reader");
      if (next.length === prev.length) return prev;
      setActiveTabId((cur) => pickActiveId(next, cur));
      return next;
    });
  }, [hasProjectPath, activeSessionId]);

  useEffect(() => {
    if (skipPersistRef.current) {
      skipPersistRef.current = false;
      return;
    }
    if (boundSessionRef.current !== activeSessionId) return;
    persistTabs(activeSessionId, tabs, activeTabId);
  }, [tabs, activeTabId, activeSessionId]);

  const addTab = useCallback(
    (kind: TabKind) => {
      if (kind === "reader" && !hasProjectPath) return;
      const tab: PanelTab = { id: newTabId(), kind };
      setTabs((prev) => [...prev, tab]);
      setActiveTabId(tab.id);
    },
    [hasProjectPath],
  );

  const setTabKind = useCallback(
    (id: string, kind: TabKind) => {
      if (kind === "reader" && !hasProjectPath) return;
      setTabs((prev) => prev.map((tb) => (tb.id === id ? { ...tb, kind } : tb)));
    },
    [hasProjectPath],
  );

  const closeTab = useCallback((id: string) => {
    setTabs((prev) => {
      const next = prev.filter((tb) => tb.id !== id);
      setActiveTabId((cur) => {
        if (cur !== id) return cur;
        const idx = prev.findIndex((tb) => tb.id === id);
        const fallback = next[idx] ?? next[idx - 1] ?? next[0];
        return fallback?.id ?? null;
      });
      return next;
    });
  }, []);

  return {
    tabs,
    setTabs,
    activeTabId,
    setActiveTabId,
    activeTabIdRef,
    addTab,
    setTabKind,
    closeTab,
  };
}
