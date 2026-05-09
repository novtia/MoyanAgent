import type { ThemeMode } from "../../theme";

export type SettingsTab = "appearance" | "llm" | "system";
export type { ThemeMode };

export interface SettingsViewProps {
  activeTab: SettingsTab;
  themeMode: ThemeMode;
  onTabChange: (tab: SettingsTab) => void;
  onThemeModeChange: (mode: ThemeMode) => void;
  onBack: () => void;
}

export interface AppInfo {
  version: string;
  data_dir: string;
  db_path: string;
  sessions_dir: string;
}
