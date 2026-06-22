import { create } from "zustand";
import type { ImportResult, ModelParamSettings, Project } from "../types";
import { api } from "../api/tauri";

interface ProjectStore {
  projects: Project[];
  refreshList: () => Promise<void>;
  createBlank: (name: string) => Promise<Project>;
  createFromFolder: (name: string, path: string) => Promise<Project>;
  rename: (id: string, name: string) => Promise<void>;
  updatePath: (id: string, path: string | null) => Promise<void>;
  remove: (id: string) => Promise<void>;
  reorder: (orderedIds: string[]) => Promise<void>;
  updateConfig: (
    id: string,
    systemPrompt: string,
    historyTurns: number,
    llmParams: ModelParamSettings,
    contextWindow: number | null,
  ) => Promise<void>;
  exportProjects: (projectIds: string[], destPath: string) => Promise<void>;
  importArchive: (archivePath: string) => Promise<ImportResult>;
}

export const useProject = create<ProjectStore>((set, get) => ({
  projects: [],

  refreshList: async () => {
    const list = await api.listProjects();
    set({ projects: list });
  },

  createBlank: async (name) => {
    const p = await api.createProject(name, null);
    await get().refreshList();
    return p;
  },

  createFromFolder: async (name, path) => {
    const p = await api.createProject(name, path);
    await get().refreshList();
    return p;
  },

  rename: async (id, name) => {
    await api.renameProject(id, name);
    await get().refreshList();
  },

  updatePath: async (id, path) => {
    await api.updateProjectPath(id, path && path.trim() ? path.trim() : null);
    await get().refreshList();
  },

  remove: async (id) => {
    await api.deleteProject(id);
    await get().refreshList();
  },

  reorder: async (orderedIds) => {
    await api.reorderProjects(orderedIds);
    await get().refreshList();
  },

  updateConfig: async (id, systemPrompt, historyTurns, llmParams, contextWindow) => {
    await api.updateProjectConfig(id, systemPrompt, historyTurns, llmParams, contextWindow);
    await get().refreshList();
  },

  exportProjects: async (projectIds, destPath) => {
    await api.exportProjectsArchive(projectIds, destPath);
  },

  importArchive: async (archivePath) => {
    const result = await api.importArchive(archivePath);
    await get().refreshList();
    return result;
  },
}));
