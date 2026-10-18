import type { SetStateAction } from "react";
import { create } from "zustand";
import { READER_TREE_KEY } from "../components/chat/rightPanel/constants";
import {
  DEFAULT_RATIO,
  MAX_RATIO,
  MIN_RATIO,
} from "../components/chat/rightPanel/reader/constants";

/**
 * File-tree layout of one project. Every reader tab of every session in the
 * project reads and writes this same record, so the tree looks identical
 * whichever tab (or conversation) the user is in.
 */
export interface ProjectTreeState {
  showTree: boolean;
  /** Editor share of the reader split. */
  ratio: number;
  /** Expanded directory paths. */
  expanded: string[];
}

const DEFAULT_STATE: ProjectTreeState = {
  showTree: true,
  ratio: DEFAULT_RATIO,
  expanded: [],
};

export const EMPTY_EXPANDED: readonly string[] = Object.freeze([]);

function clampRatio(value: number): number {
  return Math.min(MAX_RATIO, Math.max(MIN_RATIO, value));
}

function load(): Record<string, ProjectTreeState> {
  if (typeof window === "undefined") return {};
  try {
    const raw = window.localStorage.getItem(READER_TREE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, Partial<ProjectTreeState>>;
    const out: Record<string, ProjectTreeState> = {};
    for (const [pid, v] of Object.entries(parsed ?? {})) {
      if (!v || typeof v !== "object") continue;
      out[pid] = {
        showTree: typeof v.showTree === "boolean" ? v.showTree : true,
        ratio:
          typeof v.ratio === "number" && Number.isFinite(v.ratio)
            ? clampRatio(v.ratio)
            : DEFAULT_RATIO,
        expanded: Array.isArray(v.expanded)
          ? v.expanded.filter((p): p is string => typeof p === "string")
          : [],
      };
    }
    return out;
  } catch {
    return {};
  }
}

function persist(byProject: Record<string, ProjectTreeState>) {
  try {
    window.localStorage.setItem(READER_TREE_KEY, JSON.stringify(byProject));
  } catch {
    /* in-memory state stays authoritative */
  }
}

interface ReaderTreeStore {
  byProject: Record<string, ProjectTreeState>;
  setShowTree: (projectId: string, show: boolean) => void;
  setRatio: (projectId: string, ratio: number) => void;
  setExpanded: (projectId: string, updater: SetStateAction<Set<string>>) => void;
}

export const useReaderTree = create<ReaderTreeStore>((set, get) => {
  function patch(projectId: string, next: Partial<ProjectTreeState>) {
    const prev = get().byProject[projectId] ?? DEFAULT_STATE;
    const merged = { ...prev, ...next };
    if (
      merged.showTree === prev.showTree &&
      merged.ratio === prev.ratio &&
      merged.expanded === prev.expanded &&
      get().byProject[projectId]
    ) {
      return;
    }
    const byProject = { ...get().byProject, [projectId]: merged };
    set({ byProject });
    persist(byProject);
  }

  return {
    byProject: load(),
    setShowTree: (projectId, show) => patch(projectId, { showTree: show }),
    setRatio: (projectId, ratio) => patch(projectId, { ratio: clampRatio(ratio) }),
    setExpanded: (projectId, updater) => {
      const current = new Set(get().byProject[projectId]?.expanded ?? EMPTY_EXPANDED);
      const next = typeof updater === "function" ? updater(current) : updater;
      if (next === current) return;
      const prevList = get().byProject[projectId]?.expanded ?? EMPTY_EXPANDED;
      if (next.size === prevList.length && prevList.every((p) => next.has(p))) return;
      patch(projectId, { expanded: [...next] });
    },
  };
});

/** Scroll offset of the shared tree per project (transient, never persisted). */
const scrollTops = new Map<string, number>();

export function getTreeScroll(projectId: string): number {
  return scrollTops.get(projectId) ?? 0;
}

export function setTreeScroll(projectId: string, top: number) {
  scrollTops.set(projectId, top);
}
