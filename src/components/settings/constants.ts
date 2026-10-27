import type { SettingsTab } from "./types";

export const TAB_TITLE_KEYS: Record<SettingsTab, string> = {
  appearance: "settings.tabAppearance",
  llm: "settings.tabLlm",
  rewrite: "settings.tabRewrite",
  default: "settings.tabDefault",
  search: "settings.tabSearch",
  tools: "settings.tabTools",
  system: "settings.tabSystem",
};
