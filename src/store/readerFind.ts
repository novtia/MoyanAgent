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
}

export interface ReaderFindFileSummary {
  path: string;
  name: string;
  count: number;
  firstMatchIndex: number;
}

export function summarizeFindFiles(matches: ReaderFindMatch[]): ReaderFindFileSummary[] {
  const map = new Map<string, ReaderFindFileSummary>();
  matches.forEach((match, index) => {
    const key = normalizeReaderPath(match.path);
    const existing = map.get(key);
    if (existing) {
      existing.count += 1;
    } else {
      map.set(key, {
        path: match.path,
        name: readerFileName(match.path),
        count: 1,
        firstMatchIndex: index,
      });
    }
  });
  return [...map.values()].sort((a, b) => a.name.localeCompare(b.name));
}

interface SearchTarget {
  tabId: string | null;
  path: string;
  text: string;
  inTab: boolean;
}

interface ReaderFindStore {
  open: boolean;
  showReplace: boolean;
  query: string;
  replaceWith: string;
  matchCase: boolean;
  scope: ReaderFindScope;
  matchIndex: number;
  matches: ReaderFindMatch[];
  searching: boolean;
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
): Promise<SearchTarget[]> {
  const reader = useReader.getState();
  const activeTab =
    reader.tabs.find((t) => t.id === reader.activeTabId) ?? reader.tabs[0] ?? null;

  if (scope === "file") {
    if (!activeTab) return [];
    return [
      {
        tabId: activeTab.id,
        path: activeTab.path,
        text: activeTab.text,
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
      out.push({
        tabId: target.tabId,
        path: target.path,
        start: range.start,
        end: range.end,
        line,
        column,
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
  const tab = reader.getTabByPath(match.path);
  if (!tab && reader.sessionId) {
    void (async () => {
      try {
        const file = await api.readProjectFile(reader.sessionId!, match.path);
        reader.openDoc({
          path: match.path,
          text: file.text,
          fileType: inferFileType(match.path),
          encoding: file.encoding,
          hadBom: file.hadBom,
        });
        const opened = useReader.getState().getTabByPath(match.path);
        if (opened) useReader.getState().setActiveTab(opened.id);
      } catch {
        /* ignore */
      }
    })();
    return;
  }
  if (tab && reader.activeTabId !== tab.id) {
    reader.setActiveTab(tab.id);
  }
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
  open: false,
  showReplace: false,
  query: "",
  replaceWith: "",
  matchCase: false,
  scope: "file",
  matchIndex: -1,
  matches: [],
  searching: false,

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
    const { query, matchCase, scope, open, matchIndex: prevIndex } = get();
    if (!open) return;
    const sessionId = useReader.getState().sessionId;
    set({ searching: true });
    try {
      const projectRoot = scope === "all" && sessionId ? resolveProjectRoot() : null;
      const targets = await buildSearchTargets(scope, sessionId, projectRoot);
      const matches = buildMatches(targets, query, matchCase);
      // Keep selection when possible; otherwise land on the first hit.
      let matchIndex = -1;
      if (matches.length > 0) {
        matchIndex =
          prevIndex >= 0 && prevIndex < matches.length ? prevIndex : 0;
      }
      set({
        matches,
        matchIndex,
        searching: false,
      });
    } catch {
      set({ matches: [], matchIndex: -1, searching: false });
    }
  },

  nextMatch: () => {
    const { matches, matchIndex } = get();
    if (matches.length === 0) return;
    const next = matchIndex < 0 ? 0 : (matchIndex + 1) % matches.length;
    set({ matchIndex: next });
    activateMatch(matches[next] ?? null);
  },

  prevMatch: () => {
    const { matches, matchIndex } = get();
    if (matches.length === 0) return;
    const prev =
      matchIndex <= 0 ? matches.length - 1 : matchIndex - 1;
    set({ matchIndex: prev });
    activateMatch(matches[prev] ?? null);
  },

  goToFile: (path: string) => {
    const { matches } = get();
    const key = normalizeReaderPath(path);
    const idx = matches.findIndex((m) => normalizeReaderPath(m.path) === key);
    if (idx < 0) return;
    set({ matchIndex: idx });
    activateMatch(matches[idx] ?? null);
  },

  goToMatch: (index) => {
    const { matches } = get();
    if (index < 0 || index >= matches.length) return;
    set({ matchIndex: index });
    activateMatch(matches[index] ?? null);
  },

  getActiveMatch: () => {
    const { matches, matchIndex } = get();
    if (matchIndex < 0 || matchIndex >= matches.length) return null;
    return matches[matchIndex] ?? null;
  },

  replaceCurrent: async () => {
    let { query, replaceWith, matchCase, matches, matchIndex } = get();
    if (!query || matches.length === 0) return;

    if (matchIndex < 0) {
      matchIndex = 0;
      set({ matchIndex: 0 });
      activateMatch(matches[0] ?? null);
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
      void get().refreshMatches();
      return;
    }

    const nextText = replaceRange(text, start, end, replaceWith);
    applyTextToTarget(match.path, nextText, sessionId, true);
    void get().refreshMatches();
  },

  replaceAll: async () => {
    const { query, replaceWith, matchCase, scope } = get();
    if (!query) return;
    const sessionId = useReader.getState().sessionId;
    const projectRoot = scope === "all" && sessionId ? resolveProjectRoot() : null;
    const targets = await buildSearchTargets(scope, sessionId, projectRoot);
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
    void get().refreshMatches();
  },
}));
