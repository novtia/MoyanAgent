import type { SetStateAction } from "react";
import { create } from "zustand";
import {
  clearStoredTabs,
  newTabId,
  persistTabs,
  pickActiveId,
  purgeLegacyTabStorage,
  readStoredTabs,
  type StoredTabs,
} from "../components/chat/rightPanel/storage/panelTabs";
import { persistOpen, readStoredOpen } from "../components/chat/rightPanel/storage/panelOpen";
import type {
  PanelTab,
  TabKind,
  TabViewState,
} from "../components/chat/rightPanel/types";
import {
  applyReaderPathOpsToPath,
  normalizeReaderPath,
  useReader,
  type ReaderPathOp,
} from "./reader";

function resolve<T>(updater: SetStateAction<T>, current: T): T {
  return typeof updater === "function" ? (updater as (prev: T) => T)(current) : updater;
}

function sameKey(a: string | null | undefined, b: string | null | undefined): boolean {
  if (!a || !b) return false;
  return normalizeReaderPath(a) === normalizeReaderPath(b);
}

/**
 * Right-panel chrome: the single source of truth for which tabs exist, their
 * order, which one is active, and each tab's own view state.
 *
 * The store is bound to exactly one session at a time. Every mutation writes
 * through to that session's storage key, so a session switch can never carry
 * tabs (or tab view state) from another conversation.
 */
interface RightPanelStore {
  sessionId: string | null;
  tabs: PanelTab[];
  activeTabId: string | null;
  open: boolean;
  /** Bumped when a tab was opened through an explicit "reveal" intent. */
  revealSeq: number;

  bindSession: (sessionId: string | null) => void;
  /** Drop in-memory tabs if bound, and delete this session's persisted chrome. */
  forgetSession: (sessionId: string) => void;
  setOpen: (open: SetStateAction<boolean>) => void;
  selectTab: (id: string) => void;
  addTab: (kind: TabKind, opts?: { path?: string | null }) => string;
  setTabKind: (id: string, kind: TabKind) => void;
  closeTab: (id: string) => void;
  closeOtherTabs: (id: string) => void;
  closeTabsToRight: (id: string) => void;
  closeAllTabs: () => void;
  /** Merge a patch into one tab's private view state. */
  updateTabView: (id: string, patch: Partial<TabViewState>) => void;
  /** Point one reader tab at another file (back/forward navigation). */
  setTabPath: (id: string, path: string) => void;
  /** Open (or focus) a reader tab for `path` in the bound session. */
  openFileTab: (path: string, opts?: { reveal?: boolean }) => void;
  /** Apply filesystem rename/delete ops to this session's reader tabs. */
  applyPathOps: (ops: ReaderPathOp[]) => void;
}

purgeLegacyTabStorage();

/**
 * Tabs of every session touched in this run, kept in memory.
 *
 * localStorage is only the cross-restart backup: it can fail (quota, locked
 * profile) and must never be the reason a session switch loses its tabs.
 */
const snapshots = new Map<string, StoredTabs>();

export const useRightPanel = create<RightPanelStore>((set, get) => {
  /** Commit to state, the in-memory snapshot and (best effort) storage. */
  function commit(next: { tabs?: PanelTab[]; activeTabId?: string | null }) {
    const tabs = next.tabs ?? get().tabs;
    const activeTabId =
      next.activeTabId !== undefined ? next.activeTabId : get().activeTabId;
    if (tabs === get().tabs && activeTabId === get().activeTabId) return;
    const sessionId = get().sessionId;
    if (sessionId) {
      snapshots.set(sessionId, { tabs, activeId: activeTabId });
      persistTabs(sessionId, tabs, activeTabId);
    }
    set({ tabs, activeTabId });
  }

  return {
    sessionId: null,
    tabs: [],
    activeTabId: null,
    open: readStoredOpen(),
    revealSeq: 0,

    bindSession: (sessionId) => {
      const prev = get().sessionId;
      if (prev === sessionId) return;
      if (prev) {
        snapshots.set(prev, { tabs: get().tabs, activeId: get().activeTabId });
        persistTabs(prev, get().tabs, get().activeTabId);
      }
      if (!sessionId) {
        set({ sessionId: null, tabs: [], activeTabId: null });
        return;
      }
      // Memory first: it is authoritative for this run. Storage only answers
      // for sessions not yet opened since launch.
      const loaded = snapshots.get(sessionId) ?? readStoredTabs(sessionId);
      snapshots.set(sessionId, loaded);
      set({
        sessionId,
        tabs: loaded.tabs,
        activeTabId: pickActiveId(loaded.tabs, loaded.activeId),
      });
    },

    forgetSession: (sessionId) => {
      snapshots.delete(sessionId);
      if (get().sessionId === sessionId) {
        set({ sessionId: null, tabs: [], activeTabId: null });
      }
      clearStoredTabs(sessionId);
    },

    setOpen: (updater) => {
      const open = resolve(updater, get().open);
      if (open === get().open) return;
      persistOpen(open);
      set({ open });
    },

    selectTab: (id) => {
      if (id === get().activeTabId) return;
      if (!get().tabs.some((tb) => tb.id === id)) return;
      commit({ activeTabId: id });
    },

    addTab: (kind, opts) => {
      const tab: PanelTab = { id: newTabId(), kind, path: opts?.path ?? null };
      commit({ tabs: [...get().tabs, tab], activeTabId: tab.id });
      return tab.id;
    },

    setTabKind: (id, kind) => {
      const prev = get().tabs;
      const target = prev.find((tb) => tb.id === id);
      if (!target || target.kind === kind) return;
      // A kind switch restarts the tab: its old view state no longer applies.
      commit({
        tabs: prev.map((tb) => (tb.id === id ? { id: tb.id, kind, path: null } : tb)),
      });
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
      commit({ tabs: next, activeTabId });
    },

    closeOtherTabs: (id) => {
      const keep = get().tabs.find((tb) => tb.id === id);
      if (!keep) return;
      commit({ tabs: [keep], activeTabId: id });
    },

    closeTabsToRight: (id) => {
      const prev = get().tabs;
      const idx = prev.findIndex((tb) => tb.id === id);
      if (idx < 0 || idx === prev.length - 1) return;
      const next = prev.slice(0, idx + 1);
      const activeTabId = next.some((tb) => tb.id === get().activeTabId)
        ? get().activeTabId
        : id;
      commit({ tabs: next, activeTabId });
    },

    closeAllTabs: () => {
      if (get().tabs.length === 0) return;
      commit({ tabs: [], activeTabId: null });
    },

    updateTabView: (id, patch) => {
      const prev = get().tabs;
      const target = prev.find((tb) => tb.id === id);
      if (!target) return;
      const view = { ...(target.view ?? {}), ...patch };
      if (JSON.stringify(target.view ?? {}) === JSON.stringify(view)) return;
      commit({ tabs: prev.map((tb) => (tb.id === id ? { ...tb, view } : tb)) });
    },

    setTabPath: (id, path) => {
      if (!path) return;
      const prev = get().tabs;
      const target = prev.find((tb) => tb.id === id);
      if (!target || target.kind !== "reader" || sameKey(target.path, path)) return;
      commit({
        tabs: prev.map((tb) => (tb.id === id ? { ...tb, path } : tb)),
        activeTabId: id,
      });
    },

    openFileTab: (path, opts) => {
      if (!path) return;
      const reveal = opts?.reveal === true;
      const tabs = get().tabs;
      const activeTabId = get().activeTabId;

      const existing = tabs.find((tb) => tb.kind === "reader" && sameKey(tb.path, path));
      if (existing) {
        commit({ activeTabId: existing.id });
      } else {
        const active = tabs.find((tb) => tb.id === activeTabId) ?? null;
        // Deterministic reuse of the tab the user is looking at: a fresh
        // "new tab" or a reader tab that has no file yet. Never any other slot.
        if (active && (active.kind === "empty" || (active.kind === "reader" && !active.path))) {
          commit({
            tabs: tabs.map((tb) =>
              tb.id === active.id ? { ...tb, kind: "reader" as const, path } : tb,
            ),
            activeTabId: active.id,
          });
        } else {
          const tab: PanelTab = { id: newTabId(), kind: "reader", path };
          const idx = tabs.findIndex((tb) => tb.id === activeTabId);
          const next =
            idx >= 0
              ? [...tabs.slice(0, idx + 1), tab, ...tabs.slice(idx + 1)]
              : [...tabs, tab];
          commit({ tabs: next, activeTabId: tab.id });
        }
      }

      if (reveal) set({ revealSeq: get().revealSeq + 1 });
    },

    applyPathOps: (ops) => {
      if (ops.length === 0 || get().tabs.length === 0) return;
      const prev = get().tabs;
      const next: PanelTab[] = [];
      const seen = new Set<string>();
      let changed = false;
      let activatePath: string | null = null;

      for (const tb of prev) {
        if (tb.kind !== "reader" || !tb.path) {
          next.push(tb);
          continue;
        }
        const rewritten = applyReaderPathOpsToPath(tb.path, ops);
        if (rewritten == null) {
          // Deleted on disk — the tab has nothing left to show.
          changed = true;
          continue;
        }
        const key = normalizeReaderPath(rewritten);
        if (seen.has(key)) {
          // Two tabs collapsed onto the same destination after a rename.
          changed = true;
          continue;
        }
        seen.add(key);
        // A tab that kept its own path may still have visited a renamed file.
        const view = rewriteHistory(tb.view, ops);
        if (rewritten === tb.path && view === tb.view) {
          next.push(tb);
          continue;
        }
        changed = true;
        if (rewritten !== tb.path && tb.id === get().activeTabId) activatePath = rewritten;
        next.push({ ...tb, path: rewritten, view });
      }

      if (!changed) return;

      let activeTabId = get().activeTabId;
      if (activatePath) {
        activeTabId =
          next.find((tb) => tb.kind === "reader" && sameKey(tb.path, activatePath))?.id ??
          activeTabId;
      }
      commit({ tabs: next, activeTabId: pickActiveId(next, activeTabId) });
    },
  };
});

/** Rewrite a tab's nav history after files were renamed / deleted. */
function rewriteHistory(
  view: TabViewState | undefined,
  ops: ReaderPathOp[],
): TabViewState | undefined {
  const history = view?.history;
  if (!history || history.stack.length === 0) return view;
  const stack: string[] = [];
  for (const p of history.stack) {
    const rewritten = applyReaderPathOpsToPath(p, ops);
    if (!rewritten) continue;
    if (stack.some((x) => sameKey(x, rewritten))) continue;
    stack.push(rewritten);
  }
  const unchanged =
    stack.length === history.stack.length && stack.every((p, i) => p === history.stack[i]);
  if (unchanged) return view;
  const index = stack.length === 0 ? -1 : Math.min(history.index, stack.length - 1);
  return { ...view, history: { stack, index } };
}

/**
 * Open a file as a reader tab in the panel. `reveal` marks user/agent intent
 * to look at it now (the panel un-collapses and the tab is focused); passive
 * loads must not reveal, or restoring a session would pop the panel open.
 *
 * Callers that resolved the path asynchronously should pass the `sessionId`
 * they computed it for: the tab is dropped if the user moved on since.
 */
export function openFileInPanel(
  path: string,
  opts?: { reveal?: boolean; sessionId?: string | null },
) {
  if (opts?.sessionId !== undefined && useRightPanel.getState().sessionId !== opts.sessionId) {
    return;
  }
  useRightPanel.getState().openFileTab(path, { reveal: opts?.reveal });
}

// Filesystem rename/delete events flow one way: the reader store publishes
// path ops, the panel rewrites or closes its own tabs. Subscribing here (not
// in a component effect) keeps it correct while the panel is closed.
let lastPathSeq = useReader.getState().pathSeq;
useReader.subscribe((state) => {
  if (state.pathSeq === lastPathSeq) return;
  lastPathSeq = state.pathSeq;
  const ops = state.lastPathOps;
  if (ops.length > 0) useRightPanel.getState().applyPathOps(ops);
});
