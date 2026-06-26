import { create } from "zustand";
import type { ProjectDirEntry } from "../types";
import { api } from "../api/tauri";

export type ClipboardMode = "copy" | "cut";

interface Clipboard {
  mode: ClipboardMode;
  paths: string[];
}

interface FileExplorerStore {
  sessionId: string | null;
  projectRoot: string | null;
  currentDir: string | null;
  entries: ProjectDirEntry[];
  selectedPath: string | null;
  selectedPaths: string[];
  clipboard: Clipboard | null;
  loading: boolean;
  error: string | null;
  bindSession: (sessionId: string | null, projectRoot: string | null) => void;
  setSelection: (path: string | null) => void;
  setSelectedPaths: (paths: string[]) => void;
  toggleSelection: (path: string) => void;
  selectRange: (path: string) => void;
  selectAll: () => void;
  clearSelection: () => void;
  setClipboard: (clipboard: Clipboard | null) => void;
  navigate: (dirPath: string) => Promise<void>;
  navigateUp: () => Promise<void>;
  refresh: () => Promise<void>;
  createDir: (name: string) => Promise<void>;
  createFile: (name: string) => Promise<void>;
  renameEntry: (path: string, newName: string) => Promise<void>;
  deleteEntry: (path: string) => Promise<void>;
  deleteEntries: (paths: string[]) => Promise<void>;
  duplicateEntries: (paths: string[]) => Promise<void>;
  dropMove: (paths: string[], toDir: string) => Promise<number>;
  pasteInto: (dir: string) => Promise<void>;
}

function pathSep(path: string): string {
  return path.includes("\\") ? "\\" : "/";
}

export function joinPath(dir: string, name: string): string {
  const sep = pathSep(dir);
  const base = dir.replace(/[/\\]+$/, "");
  const trimmed = name.trim().replace(/^[/\\]+|[/\\]+$/g, "");
  return `${base}${sep}${trimmed}`;
}

export function parentDir(path: string): string | null {
  const normalized = path.replace(/[/\\]+$/, "");
  const idx = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"));
  if (idx <= 0) return null;
  return normalized.slice(0, idx);
}

export function baseName(path: string): string {
  const normalized = path.replace(/[/\\]+$/, "");
  const idx = Math.max(normalized.lastIndexOf("/"), normalized.lastIndexOf("\\"));
  return idx >= 0 ? normalized.slice(idx + 1) : normalized;
}

export function siblingPath(path: string, newName: string): string {
  const parent = parentDir(path);
  if (!parent) return newName.trim();
  return joinPath(parent, newName.trim());
}

function normPath(p: string): string {
  return p.replace(/[/\\]+$/, "").replace(/\\/g, "/").toLowerCase();
}

function isSamePath(a: string | null, b: string | null): boolean {
  if (!a || !b) return false;
  return normPath(a) === normPath(b);
}

/** True when `child` is `parent` itself or nested inside it. */
function isWithin(parent: string, child: string): boolean {
  const p = normPath(parent);
  const c = normPath(child);
  return c === p || c.startsWith(`${p}/`);
}

function splitNameExt(name: string): { base: string; ext: string } {
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return { base: name, ext: "" };
  return { base: name.slice(0, dot), ext: name.slice(dot) };
}

function uniqueName(existing: Set<string>, name: string): string {
  if (!existing.has(name)) return name;
  const { base, ext } = splitNameExt(name);
  let i = 2;
  while (existing.has(`${base} (${i})${ext}`)) i += 1;
  return `${base} (${i})${ext}`;
}

export function relativePathSegments(root: string, dir: string): string[] {
  const normRoot = root.replace(/[/\\]+$/, "").replace(/\\/g, "/").toLowerCase();
  const normDir = dir.replace(/[/\\]+$/, "").replace(/\\/g, "/").toLowerCase();
  if (!normDir.startsWith(normRoot)) return [];
  const rest = dir.slice(root.length).replace(/^[/\\]+/, "");
  if (!rest) return [];
  return rest.split(/[/\\]/).filter(Boolean);
}

export const useFileExplorer = create<FileExplorerStore>((set, get) => ({
  sessionId: null,
  projectRoot: null,
  currentDir: null,
  entries: [],
  selectedPath: null,
  selectedPaths: [],
  clipboard: null,
  loading: false,
  error: null,

  bindSession: (sessionId, projectRoot) => {
    const root = projectRoot?.trim() ? projectRoot.trim() : null;
    const prev = get();
    if (prev.sessionId === sessionId && prev.projectRoot === root) {
      return;
    }
    set({
      sessionId,
      projectRoot: root,
      currentDir: root,
      entries: [],
      selectedPath: null,
      selectedPaths: [],
      clipboard: null,
      error: null,
    });
    if (sessionId && root) {
      void get().refresh();
    }
  },

  setSelection: (path) =>
    set({
      selectedPath: path,
      selectedPaths: path ? [path] : [],
    }),

  setSelectedPaths: (paths) =>
    set({
      selectedPaths: paths,
      selectedPath: paths.length > 0 ? paths[paths.length - 1] : null,
    }),

  toggleSelection: (path) => {
    const { selectedPaths } = get();
    const has = selectedPaths.includes(path);
    set({
      selectedPaths: has
        ? selectedPaths.filter((p) => p !== path)
        : [...selectedPaths, path],
      selectedPath: path,
    });
  },

  selectRange: (path) => {
    const { entries, selectedPath } = get();
    const order = entries.map((e) => e.path);
    const anchor = selectedPath ?? path;
    const ai = order.indexOf(anchor);
    const bi = order.indexOf(path);
    if (ai < 0 || bi < 0) {
      set({ selectedPaths: [path], selectedPath: path });
      return;
    }
    const [lo, hi] = ai <= bi ? [ai, bi] : [bi, ai];
    set({ selectedPaths: order.slice(lo, hi + 1) });
  },

  selectAll: () => {
    const { entries } = get();
    set({ selectedPaths: entries.map((e) => e.path) });
  },

  clearSelection: () => set({ selectedPath: null, selectedPaths: [] }),

  setClipboard: (clipboard) => set({ clipboard }),

  navigate: async (dirPath) => {
    set({ currentDir: dirPath, selectedPath: null, selectedPaths: [] });
    await get().refresh();
  },

  navigateUp: async () => {
    const { currentDir, projectRoot } = get();
    if (!currentDir || !projectRoot) return;
    const parent = parentDir(currentDir);
    if (!parent || parent.length < projectRoot.length) {
      await get().navigate(projectRoot);
      return;
    }
    await get().navigate(parent);
  },

  refresh: async () => {
    const { sessionId, currentDir, projectRoot } = get();
    if (!sessionId || !projectRoot) {
      set({ entries: [], loading: false, error: null });
      return;
    }
    set({ loading: true, error: null });
    try {
      const entries = await api.listProjectDir(sessionId, currentDir ?? projectRoot);
      set({ entries, loading: false });
    } catch (err) {
      set({
        entries: [],
        loading: false,
        error: err instanceof Error ? err.message : String(err),
      });
    }
  },

  createDir: async (name) => {
    const { sessionId, currentDir, projectRoot } = get();
    if (!sessionId || !currentDir || !projectRoot) return;
    const path = joinPath(currentDir, name);
    await api.createProjectDir(sessionId, path);
    await get().refresh();
  },

  createFile: async (name) => {
    const { sessionId, currentDir, projectRoot } = get();
    if (!sessionId || !currentDir || !projectRoot) return;
    const path = joinPath(currentDir, name);
    await api.createProjectFile(sessionId, path, "");
    await get().refresh();
  },

  renameEntry: async (path, newName) => {
    const { sessionId } = get();
    if (!sessionId) return;
    const to = siblingPath(path, newName);
    await api.renameProjectPath(sessionId, path, to);
    const { selectedPath, selectedPaths } = get();
    set({
      selectedPath: selectedPath === path ? to : selectedPath,
      selectedPaths: selectedPaths.map((p) => (p === path ? to : p)),
    });
    await get().refresh();
  },

  deleteEntry: async (path) => {
    await get().deleteEntries([path]);
  },

  deleteEntries: async (paths) => {
    const { sessionId } = get();
    if (!sessionId || paths.length === 0) return;
    for (const path of paths) {
      await api.deleteProjectPath(sessionId, path);
    }
    const removed = new Set(paths.map((p) => normPath(p)));
    const { selectedPath, selectedPaths, clipboard } = get();
    set({
      selectedPath:
        selectedPath && removed.has(normPath(selectedPath)) ? null : selectedPath,
      selectedPaths: selectedPaths.filter((p) => !removed.has(normPath(p))),
      clipboard: clipboard
        ? {
            ...clipboard,
            paths: clipboard.paths.filter((p) => !removed.has(normPath(p))),
          }
        : null,
    });
    await get().refresh();
  },

  duplicateEntries: async (paths) => {
    const { sessionId, entries } = get();
    if (!sessionId || paths.length === 0) return;
    const existing = new Set(entries.map((e) => e.name));
    const created: string[] = [];
    for (const from of paths) {
      const parent = parentDir(from);
      if (!parent) continue;
      const finalName = uniqueName(existing, baseName(from));
      existing.add(finalName);
      const target = joinPath(parent, finalName);
      await api.copyProjectPath(sessionId, from, target);
      created.push(target);
    }
    set({
      selectedPaths: created,
      selectedPath: created[created.length - 1] ?? null,
    });
    await get().refresh();
  },

  dropMove: async (paths, toDir) => {
    const { sessionId } = get();
    if (!sessionId || paths.length === 0) return 0;
    let moved = 0;
    for (const from of paths) {
      if (isSamePath(parentDir(from), toDir)) continue;
      if (isWithin(from, toDir)) continue;
      const target = joinPath(toDir, baseName(from));
      await api.renameProjectPath(sessionId, from, target);
      moved += 1;
    }
    set({ selectedPath: null, selectedPaths: [] });
    if (moved > 0) await get().refresh();
    return moved;
  },

  pasteInto: async (dir) => {
    const { sessionId, clipboard, entries, currentDir } = get();
    if (!sessionId || !clipboard || clipboard.paths.length === 0) return;
    const intoCurrent = isSamePath(dir, currentDir);
    const existing = new Set(intoCurrent ? entries.map((e) => e.name) : []);
    const pasted: string[] = [];
    for (const from of clipboard.paths) {
      const name = baseName(from);
      if (clipboard.mode === "cut") {
        if (isSamePath(parentDir(from), dir)) continue;
        if (isWithin(from, dir)) continue;
        const finalName = uniqueName(existing, name);
        existing.add(finalName);
        const target = joinPath(dir, finalName);
        await api.renameProjectPath(sessionId, from, target);
        pasted.push(target);
      } else {
        if (isWithin(from, dir) && !intoCurrent) continue;
        const finalName = uniqueName(existing, name);
        existing.add(finalName);
        const target = joinPath(dir, finalName);
        await api.copyProjectPath(sessionId, from, target);
        pasted.push(target);
      }
    }
    set({
      clipboard: clipboard.mode === "cut" ? null : clipboard,
      selectedPath: null,
      selectedPaths: intoCurrent ? pasted : [],
    });
    await get().refresh();
  },
}));
