import { create } from "zustand";
import type { ProjectDirEntry } from "../types";
import { api } from "../api/tauri";

interface FileExplorerStore {
  sessionId: string | null;
  projectRoot: string | null;
  currentDir: string | null;
  entries: ProjectDirEntry[];
  selectedPath: string | null;
  loading: boolean;
  error: string | null;
  bindSession: (sessionId: string | null, projectRoot: string | null) => void;
  setSelectedPath: (path: string | null) => void;
  navigate: (dirPath: string) => Promise<void>;
  navigateUp: () => Promise<void>;
  refresh: () => Promise<void>;
  createDir: (name: string) => Promise<void>;
  createFile: (name: string) => Promise<void>;
  renameEntry: (path: string, newName: string) => Promise<void>;
  deleteEntry: (path: string) => Promise<void>;
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

export function siblingPath(path: string, newName: string): string {
  const parent = parentDir(path);
  if (!parent) return newName.trim();
  return joinPath(parent, newName.trim());
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
      error: null,
    });
    if (sessionId && root) {
      void get().refresh();
    }
  },

  setSelectedPath: (path) => set({ selectedPath: path }),

  navigate: async (dirPath) => {
    set({ currentDir: dirPath, selectedPath: null });
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
    const { selectedPath } = get();
    if (selectedPath === path) {
      set({ selectedPath: to });
    }
    await get().refresh();
  },

  deleteEntry: async (path) => {
    const { sessionId, selectedPath } = get();
    if (!sessionId) return;
    await api.deleteProjectPath(sessionId, path);
    if (selectedPath === path) {
      set({ selectedPath: null });
    }
    await get().refresh();
  },
}));
