import { create } from "zustand";
import type { Settings, SettingsPatch } from "../types";
import { api } from "../api/tauri";

interface SettingsStore {
  settings: Settings | null;
  loading: boolean;
  load: () => Promise<void>;
  update: (patch: SettingsPatch) => Promise<void>;
}

export const useSettings = create<SettingsStore>((set, get) => ({
  settings: null,
  loading: false,
  load: async () => {
    set({ loading: true });
    try {
      const s = await api.getSettings();
      set({ settings: s, loading: false });
    } catch (e) {
      console.error(e);
      set({ loading: false });
    }
  },
  update: async (patch) => {
    const s = await api.updateSettings(patch);
    set({ settings: s });
  },
}));
