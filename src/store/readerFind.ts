import { create } from "zustand";
import { api } from "../api/tauri";
import {
  findInText,
  isSearchableTextFile,
  lineColumnAt,
  PROJECT_SEARCH_FILE_CAP,
  replaceRange,
  resolveFindScrollIndex,
} from "../utils/readerFind";
import { normalizeReaderPath, readerFileName, useReader, inferFileType } from "./reader";
import { useProject } from "./project";
import { openFileInPanel, useRightPanel } from "./rightPanel";
import { useSession } from "./session";

export type ReaderFindScope = "file" | "all";

/** Chrome inset (px) reserved above the editor for the find bar + file list. */
export const READER_CHROME_INSET = {
  closed: 0,
  bar: 92,
  barWithList: 236,
} as const;

/** Derive the chrome-top inset from discrete find-bar state.
 *  Returns one of READER_CHROME_INSET based on open / scope / query / searching. */
export function selectReaderChromeInset(s: {
  open: boolean;
  scope: ReaderFindScope;
  query: string;
  searching: boolean;
}): number {
  if (!s.open) return READER_CHROME_INSET.closed;
  const showFileList =
    s.scope === "all" && s.query.trim().length > 0 && !s.searching;
  return showFileList ? READER_CHROME_INSET.barWithList : READER_CHROME_INSET.bar;
}

export interface ReaderFindMatch {
  tabId: string | null;
  path: string;
  start: number;
  end: number;
  line: number;
  column: number;
  /** Line text at the match (for result list snippets). */
  snippet: string;
  /** Match start/end offsets within `snippet`. */
  snippetStart: number;
  snippetEnd: number;
}

export interface ReaderFindFileSummary {
  path: string;
  name: string;
  count: number;
  firstMatchIndex: number;
}

export interface ReaderFindFileGroup extends ReaderFindFileSummary {
  matchIndexes: number[];
}

export function summarizeFindFiles(matches: ReaderFindMatch[]): ReaderFindFileSummary[] {
  return groupFindMatches(matches).map(({ matchIndexes: _i, ...summary }) => summary);
}

/** Group matches by file in encounter order (already sorted by filename in build). */
export function groupFindMatches(matches: ReaderFindMatch[]): ReaderFindFileGroup[] {
  const groups: ReaderFindFileGroup[] = [];
  const map = new Map<string, ReaderFindFileGroup>();
  matches.forEach((match, index) => {
    const key = normalizeReaderPath(match.path);
    const existing = map.get(key);
    if (existing) {
      existing.count += 1;
      existing.matchIndexes.push(index);
    } else {
      const group: ReaderFindFileGroup = {
        path: match.path,
        name: readerFileName(match.path),
        count: 1,
        firstMatchIndex: index,
        matchIndexes: [index],
      };
      map.set(key, group);
      groups.push(group);
    }
  });
  return groups;
}

function snippetForRange(
  text: string,
  start: number,
  end: number,
): { snippet: string; snippetStart: number; snippetEnd: number } {
  const lineStart = text.lastIndexOf("\n", Math.max(0, start - 1)) + 1;
  const lineEndIdx = text.indexOf("\n", start);
  const lineText = text.slice(lineStart, lineEndIdx < 0 ? text.length : lineEndIdx);
  const snippetStart = Math.max(0, start - lineStart);
  const snippetEnd = Math.max(snippetStart, end - lineStart);
  return {
    snippet: lineText.length > 0 ? lineText : text.slice(start, end),
    snippetStart,
    snippetEnd,
  };
}

interface SearchTarget {
  tabId: string | null;
  path: string;
  text: string;
  inTab: boolean;
}

/** The slice of find state that belongs to one panel tab. */
interface FindSlice {
  open: boolean;
  query: string;
  replaceWith: string;
  matchCase: boolean;
  scope: ReaderFindScope;
}

const DEFAULT_SLICE: FindSlice = {
  open: false,
  query: "",
  replaceWith: "",
  matchCase: false,
  scope: "file",
};

interface ReaderFindStore {
  /** Panel tab whose find state is currently loaded (null = none bound). */
  boundTabId: string | null;
  open: boolean;
  showReplace: boolean;
  query: string;
  replaceWith: string;
  matchCase: boolean;
  scope: ReaderFindScope;
  matchIndex: number;
  /** Bumps on every navigation so the editor re-scrolls even when index is unchanged. */
  navEpoch: number;
  matches: ReaderFindMatch[];
  searching: boolean;
  /** Swap in the find state owned by `tabId` (saving the outgoing tab's). */
  bindTab: (tabId: string | null) => void;
  /** Forget every tab's find state (session switch). */
  reset: () => void;
  openFind: (opts?: { replace?: boolean }) => void;
  close: () => void;
  setQuery: (query: string) => void;
  setReplaceWith: (value: string) => void;
  setMatchCase: (value: boolean) => void;
  setScope: (scope: ReaderFindScope) => void;
  refreshMatches: () => Promise<void>;
  nextMatch: () => void;
  prevMatch: () => void;
  goToFile: (path: string) => void;
  goToMatch: (index: number) => void;
  replaceCurrent: () => Promise<void>;
  replaceAll: () => Promise<void>;
  getActiveMatch: () => ReaderFindMatch | null;
}

async function collectProjectTextFiles(
  sessionId: string,
  root: string,
): Promise<string[]> {
  const files: string[] = [];
  const dirs = [root];
  while (dirs.length > 0 && files.length < PROJECT_SEARCH_FILE_CAP) {
    const dir = dirs.pop()!;
    let entries;
    try {
      entries = await api.listProjectDir(sessionId, dir);
    } catch {
      continue;
    }
    for (const entry of entries) {
      if (files.length >= PROJECT_SEARCH_FILE_CAP) break;
      if (entry.isDir) {
        dirs.push(entry.path);
      } else if (isSearchableTextFile(entry.path)) {
        files.push(entry.path);
      }
    }
  }
  return files;
}

async function buildSearchTargets(
  scope: ReaderFindScope,
  sessionId: string | null,
  projectRoot: string | null,
  /** Path of the panel tab the find bar is bound to (file-scope target). */
  boundPath: string | null,
): Promise<SearchTarget[]> {
  const reader = useReader.getState();

  if (scope === "file") {
    const doc = boundPath ? reader.getTabByPath(boundPath) : undefined;
    if (!doc) return [];
    return [
      {
        tabId: doc.id,
        path: doc.path,
        text: doc.text,
        inTab: true,
      },
    ];
  }

  const byPath = new Map<string, SearchTarget>();
  for (const tab of reader.tabs) {
    byPath.set(normalizeReaderPath(tab.path), {
      tabId: tab.id,
      path: tab.path,
      text: tab.text,
      inTab: true,
    });
  }

  if (sessionId && projectRoot) {
    const paths = await collectProjectTextFiles(sessionId, projectRoot);
    for (const path of paths) {
      const key = normalizeReaderPath(path);
      if (byPath.has(key)) continue;
      try {
        const file = await api.readProjectFile(sessionId, path);
        byPath.set(key, { tabId: null, path, text: file.text, inTab: false });
      } catch {
        /* skip unreadable files */
      }
    }
  }

  return [...byPath.values()].sort((a, b) =>
    readerFileName(a.path).localeCompare(readerFileName(b.path)),
  );
}

function buildMatches(
  targets: SearchTarget[],
  query: string,
  matchCase: boolean,
): ReaderFindMatch[] {
  if (!query) return [];
  const out: ReaderFindMatch[] = [];
  for (const target of targets) {
    for (const range of findInText(target.text, query, matchCase)) {
      const { line, column } = lineColumnAt(target.text, range.start);
      const { snippet, snippetStart, snippetEnd } = snippetForRange(
        target.text,
        range.start,
        range.end,
      );
      out.push({
        tabId: target.tabId,
        path: target.path,
        start: range.start,
        end: range.end,
        line,
        column,
        snippet,
        snippetStart,
        snippetEnd,
      });
    }
  }
  return out;
}

function resolveProjectRoot(): string | null {
  const active = useSession.getState().active;
  const pid = active?.session.project_id;
  if (!pid) return null;
  return useProject.getState().projects.find((p) => p.id === pid)?.path?.trim() ?? null;
}

function activateMatch(match: ReaderFindMatch | null) {
  if (!match) return;
  const reader = useReader.getState();
  const sessionId = reader.sessionId;

  // Focus the panel tab for this file (creating one when the match came from a
  // project-wide search). When that moves focus to a different tab, the
  // in-progress query travels with it instead of being replaced by that tab's
  // own last search.
  const panel = useRightPanel.getState();
  const key = normalizeReaderPath(match.path);
  const target = panel.tabs.find(
    (tb) => tb.kind === "reader" && !!tb.path && normalizeReaderPath(tb.path) === key,
  );
  carrySlice =
    !target || target.id !== panel.activeTabId ? sliceOf(useReaderFind.getState()) : null;
  openFileInPanel(match.path, { reveal: true, sessionId });

  if (!reader.getTabByPath(match.path) && sessionId) {
    void (async () => {
      try {
        const file = await api.readProjectFile(sessionId, match.path);
        if (useReader.getState().sessionId !== sessionId) return;
        useReader.getState().openDoc({
          path: match.path,
          text: file.text,
          fileType: inferFileType(match.path),
          encoding: file.encoding,
          hadBom: file.hadBom,
        });
      } catch {
        /* ignore */
      }
    })();
  }
}

function sliceOf(s: FindSlice): FindSlice {
  return {
    open: s.open,
    query: s.query,
    replaceWith: s.replaceWith,
    matchCase: s.matchCase,
    scope: s.scope,
  };
}

/** In-memory find state per panel tab (mirrored into the tab's view state). */
const slices = new Map<string, FindSlice>();
/**
 * Set right before find navigation moves focus to another tab, so the newly
 * bound tab adopts the running search instead of its own stale slice.
 */
let carrySlice: FindSlice | null = null;

function loadSlice(tabId: string): FindSlice {
  const mem = slices.get(tabId);
  if (mem) return mem;
  const stored = useRightPanel.getState().tabs.find((tb) => tb.id === tabId)?.view?.find;
  return stored ? { ...DEFAULT_SLICE, ...stored } : DEFAULT_SLICE;
}

/** File-scope search target: the path held by the bound panel tab. */
function boundTabPath(tabId: string | null): string | null {
  if (!tabId) return null;
  const tab = useRightPanel.getState().tabs.find((tb) => tb.id === tabId);
  return tab?.kind === "reader" ? (tab.path ?? null) : null;
}

function resolveMatchIndexAfterRefresh(
  matches: ReaderFindMatch[],
  prev: ReaderFindMatch | null,
  prevIndex: number,
): number {
  if (matches.length === 0) return -1;
  if (prev) {
    const prevKey = normalizeReaderPath(prev.path);
    const exact = matches.findIndex(
      (m) =>
        normalizeReaderPath(m.path) === prevKey &&
        m.start === prev.start &&
        m.end === prev.end,
    );
    if (exact >= 0) return exact;
    // After replace/edit: land on the next hit at/after the old offset.
    const after = matches.findIndex(
      (m) => normalizeReaderPath(m.path) === prevKey && m.start >= prev.start,
    );
    if (after >= 0) return after;
    const samePathLast = [...matches]
      .map((m, i) => ({ m, i }))
      .reverse()
      .find(({ m }) => normalizeReaderPath(m.path) === prevKey);
    if (samePathLast) return samePathLast.i;
    // Previous file is no longer in this result set (scope/tab change).
    return -1;
  }
  // No prior selection (fresh query): stay unset so the first Enter lands on #1.
  if (prevIndex < 0) return -1;
  if (prevIndex < matches.length) return prevIndex;
  return 0;
}

function applyTextToTarget(
  path: string,
  text: string,
  sessionId: string | null,
  dirty = true,
) {
  const reader = useReader.getState();
  const tab = reader.getTabByPath(path);
  if (tab) {
    reader.updateTabText(path, text, { dirty });
    if (sessionId) {
      void api.writeProjectFile(sessionId, path, text, tab.encoding, tab.hadBom);
    }
    return;
  }
  if (sessionId) {
    void api.writeProjectFile(sessionId, path, text);
  }
}

export const useReaderFind = create<ReaderFindStore>((set, get) => ({
  boundTabId: null,
  open: false,
  showReplace: false,
  query: "",
  replaceWith: "",
  matchCase: false,
  scope: "file",
  matchIndex: -1,
  navEpoch: 0,
  matches: [],
  searching: false,

  bindTab: (tabId) => {
    const prevTabId = get().boundTabId;
    if (prevTabId === tabId) {
      carrySlice = null;
      return;
    }
    if (prevTabId) {
      const slice = sliceOf(get());
      slices.set(prevTabId, slice);
      useRightPanel.getState().updateTabView(prevTabId, { find: slice });
    }
    const carried = carrySlice;
    carrySlice = null;
    if (!tabId) {
      set({
        boundTabId: null,
        ...DEFAULT_SLICE,
        showReplace: false,
        matchIndex: -1,
        matches: [],
        searching: false,
      });
      return;
    }
    if (carried) {
      // Find navigation moved focus to this tab: same search, so keep the
      // result list and the position the user just navigated to.
      set({ boundTabId: tabId, ...carried });
      return;
    }
    const next = loadSlice(tabId);
    set({
      boundTabId: tabId,
      ...next,
      showReplace: false,
      matchIndex: -1,
      matches: [],
      searching: false,
    });
    if (next.open && next.query) void get().refreshMatches();
  },

  reset: () => {
    slices.clear();
    carrySlice = null;
    set({
      boundTabId: null,
      ...DEFAULT_SLICE,
      showReplace: false,
      matchIndex: -1,
      matches: [],
      searching: false,
    });
  },

  openFind: (opts) => {
    set({
      open: true,
      showReplace: opts?.replace === true,
      matchIndex: -1,
    });
    void get().refreshMatches();
  },

  close: () => {
    set({
      open: false,
      showReplace: false,
      matchIndex: -1,
      matches: [],
    });
    const tabId = get().boundTabId;
    if (tabId) {
      const slice = sliceOf(get());
      slices.set(tabId, slice);
      useRightPanel.getState().updateTabView(tabId, { find: slice });
    }
  },

  setQuery: (query) => {
    set({ query, matchIndex: -1 });
    void get().refreshMatches();
  },

  setReplaceWith: (replaceWith) => set({ replaceWith }),

  setMatchCase: (matchCase) => {
    set({ matchCase, matchIndex: -1 });
    void get().refreshMatches();
  },

  setScope: (scope) => {
    set({ scope, matchIndex: -1 });
    void get().refreshMatches();
  },

  refreshMatches: async () => {
    const { query, matchCase, scope, open, matchIndex: prevIndex, navEpoch } = get();
    if (!open) return;
    const prev = get().getActiveMatch();
    const sessionId = useReader.getState().sessionId;
    const tabId = get().boundTabId;
    set({ searching: true });
    try {
      const projectRoot = scope === "all" && sessionId ? resolveProjectRoot() : null;
      const targets = await buildSearchTargets(scope, sessionId, projectRoot, boundTabPath(tabId));
      // Bail out if the user switched tab / session while we were scanning:
      // those results belong to a search the user has already left behind.
      if (get().boundTabId !== tabId || useReader.getState().sessionId !== sessionId) {
        set({ searching: false });
        return;
      }
      const matches = buildMatches(targets, query, matchCase);
      const matchIndex = resolveMatchIndexAfterRefresh(matches, prev, prevIndex);
      set({
        matches,
        matchIndex,
        searching: false,
        // Re-scroll to the resolved hit (and avoid first Enter skipping #1).
        navEpoch: matchIndex >= 0 ? navEpoch + 1 : navEpoch,
      });
      if (matchIndex >= 0) {
        activateMatch(matches[matchIndex] ?? null);
      }
    } catch {
      set({ matches: [], matchIndex: -1, searching: false });
    }
  },

  nextMatch: () => {
    const { matches, matchIndex, navEpoch } = get();
    if (matches.length === 0) return;
    const next = matchIndex < 0 ? 0 : (matchIndex + 1) % matches.length;
    set({ matchIndex: next, navEpoch: navEpoch + 1 });
    activateMatch(matches[next] ?? null);
  },

  prevMatch: () => {
    const { matches, matchIndex, navEpoch } = get();
    if (matches.length === 0) return;
    const prev =
      matchIndex <= 0 ? matches.length - 1 : matchIndex - 1;
    set({ matchIndex: prev, navEpoch: navEpoch + 1 });
    activateMatch(matches[prev] ?? null);
  },

  goToFile: (path: string) => {
    const { matches, navEpoch } = get();
    const key = normalizeReaderPath(path);
    const idx = matches.findIndex((m) => normalizeReaderPath(m.path) === key);
    if (idx < 0) return;
    set({ matchIndex: idx, navEpoch: navEpoch + 1 });
    activateMatch(matches[idx] ?? null);
  },

  goToMatch: (index) => {
    const { matches, navEpoch } = get();
    if (index < 0 || index >= matches.length) return;
    set({ matchIndex: index, navEpoch: navEpoch + 1 });
    activateMatch(matches[index] ?? null);
  },

  getActiveMatch: () => {
    const { matches, matchIndex } = get();
    if (matchIndex < 0 || matchIndex >= matches.length) return null;
    return matches[matchIndex] ?? null;
  },

  replaceCurrent: async () => {
    let { query, replaceWith, matchCase, matches, matchIndex, navEpoch } = get();
    if (!query || matches.length === 0) return;

    if (matchIndex < 0) {
      matchIndex = 0;
      set({ matchIndex: 0, navEpoch: navEpoch + 1 });
      activateMatch(matches[0] ?? null);
      navEpoch = get().navEpoch;
    }

    const match = matches[matchIndex] ?? null;
    if (!match) return;

    const sessionId = useReader.getState().sessionId;
    const reader = useReader.getState();
    const tab = reader.getTabByPath(match.path);
    const text =
      tab?.text ??
      (sessionId ? (await api.readProjectFile(sessionId, match.path)).text : "");
    const scrollIndex = resolveFindScrollIndex(
      text,
      query,
      matchCase,
      match,
      matches.filter((m) => normalizeReaderPath(m.path) === normalizeReaderPath(match.path)),
    );
    const start = scrollIndex ?? match.start;
    const end = start + query.length;
    const actual = text.slice(start, end);
    const expected = matchCase ? query : query.toLowerCase();
    const found = matchCase ? actual : actual.toLowerCase();
    if (found !== expected) {
      await get().refreshMatches();
      return;
    }

    const nextText = replaceRange(text, start, end, replaceWith);
    applyTextToTarget(match.path, nextText, sessionId, true);
    // refreshMatches restores index by identity / next-at-offset and re-scrolls.
    await get().refreshMatches();
  },

  replaceAll: async () => {
    const { query, replaceWith, matchCase, scope } = get();
    if (!query) return;
    const sessionId = useReader.getState().sessionId;
    const projectRoot = scope === "all" && sessionId ? resolveProjectRoot() : null;
    const targets = await buildSearchTargets(
      scope,
      sessionId,
      projectRoot,
      boundTabPath(get().boundTabId),
    );
    for (const target of targets) {
      const ranges = findInText(target.text, query, matchCase);
      if (ranges.length === 0) continue;
      let text = target.text;
      for (let i = ranges.length - 1; i >= 0; i--) {
        const r = ranges[i]!;
        text = replaceRange(text, r.start, r.end, replaceWith);
      }
      applyTextToTarget(target.path, text, sessionId, true);
    }
    await get().refreshMatches();
  },
}));

// Mirror every edit of the visible find state into the bound tab's slice, so
// switching tabs (or reopening the find bar) restores exactly that tab's search.
useReaderFind.subscribe((s, prev) => {
  const tabId = s.boundTabId;
  if (!tabId || tabId !== prev.boundTabId) return;
  if (
    s.open === prev.open &&
    s.query === prev.query &&
    s.replaceWith === prev.replaceWith &&
    s.matchCase === prev.matchCase &&
    s.scope === prev.scope
  ) {
    return;
  }
  slices.set(tabId, sliceOf(s));
});
