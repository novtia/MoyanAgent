import type { SetStateAction } from "react";
import { create } from "zustand";
import {
  clearStoredTabs,
  newTabId,
  persistTabs,
  pickActiveId,
  readStoredTabs,
} from "../components/chat/rightPanel/storage/panelTabs";
import { persistOpen, readStoredOpen } from "../components/chat/rightPanel/storage/panelOpen";
import type { PanelTab, TabKind } from "../components/chat/rightPanel/types";

function resolve<T>(updater: SetStateAction<T>, current: T): T {
  return typeof updater === "function" ? (updater as (prev: T) => T)(current) : updater;
}

interface RightPanelChromeStore {
  sessionId: string | null;
  tabs: PanelTab[];
  activeTabId: string | null;
  open: boolean;
  bindSession: (sessionId: string | null) => void;
  /** Drop in-memory tabs if bound, and delete this session's persisted chrome. */
  forgetSession: (sessionId: string) => void;
  setOpen: (open: SetStateAction<boolean>) => void;
  setTabs: (updater: SetStateAction<PanelTab[]>) => void;
  setActiveTabId: (updater: SetStateAction<string | null>) => void;
  addTab: (kind: TabKind) => void;
  setTabKind: (id: string, kind: TabKind) => void;
  closeTab: (id: string) => void;
  closeOtherTabs: (id: string) => void;
  closeTabsToRight: (id: string) => void;
  closeAllTabs: () => void;
}

export const useRightPanel = create<RightPanelChromeStore>((set, get) => ({
  sessionId: null,
  tabs: [],
  activeTabId: null,
  open: readStoredOpen(),

  bindSession: (sessionId) => {
    const prev = get().sessionId;
    if (prev === sessionId) return;
    if (prev) persistTabs(prev, get().tabs, get().activeTabId);
    if (!sessionId) {
      set({ sessionId: null, tabs: [], activeTabId: null });
      return;
    }
    const loaded = readStoredTabs(sessionId);
    set({
      sessionId,
      tabs: loaded.tabs,
      activeTabId: pickActiveId(loaded.tabs, loaded.activeId),
    });
  },

  forgetSession: (sessionId) => {
    if (get().sessionId === sessionId) {
      set({ sessionId: null, tabs: [], activeTabId: null });
    }
    clearStoredTabs(sessionId);
  },

  setOpen: (updater) => {
    const open = resolve(updater, get().open);
    persistOpen(open);
    set({ open });
  },

  setTabs: (updater) => {
    const tabs = resolve(updater, get().tabs);
    if (tabs === get().tabs) return;
    persistTabs(get().sessionId, tabs, get().activeTabId);
    set({ tabs });
  },

  setActiveTabId: (updater) => {
    const activeTabId = resolve(updater, get().activeTabId);
    if (activeTabId === get().activeTabId) return;
    persistTabs(get().sessionId, get().tabs, activeTabId);
    set({ activeTabId });
  },

  addTab: (kind) => {
    const tab: PanelTab = { id: newTabId(), kind };
    const tabs = [...get().tabs, tab];
    persistTabs(get().sessionId, tabs, tab.id);
    set({ tabs, activeTabId: tab.id });
  },

  setTabKind: (id, kind) => {
    const tabs = get().tabs.map((tb) => (tb.id === id ? { ...tb, kind } : tb));
    persistTabs(get().sessionId, tabs, get().activeTabId);
    set({ tabs });
  },

  closeTab: (id) => {
    const prev = get().tabs;
    const next = prev.filter((tb) => tb.id !== id);
    if (next.length === prev.length) return;
    let activeTabId = get().activeTabId;
    if (activeTabId === id) {
      const idx = prev.findIndex((tb) => tb.id === id);
      activeTabId = next[idx]?.id ?? next[idx - 1]?.id ?? next[0]?.id ?? null;
    }
    persistTabs(get().sessionId, next, activeTabId);
    set({ tabs: next, activeTabId });
  },

  closeOtherTabs: (id) => {
    const keep = get().tabs.find((tb) => tb.id === id);
    if (!keep) return;
    persistTabs(get().sessionId, [keep], id);
    set({ tabs: [keep], activeTabId: id });
  },

  closeTabsToRight: (id) => {
    const prev = get().tabs;
    const idx = prev.findIndex((tb) => tb.id === id);
    if (idx < 0) return;
    const next = prev.slice(0, idx + 1);
    const activeTabId =
      get().activeTabId != null && next.some((tb) => tb.id === get().activeTabId)
        ? get().activeTabId
        : id;
    persistTabs(get().sessionId, next, activeTabId);
    set({ tabs: next, activeTabId });
  },

  closeAllTabs: () => {
    persistTabs(get().sessionId, [], null);
    set({ tabs: [], activeTabId: null });
  },
}));
