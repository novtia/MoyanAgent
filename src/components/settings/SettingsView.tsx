import { useTranslation } from "react-i18next";
import { TAB_TITLE_KEYS } from "./constants";
import { AppearanceSection } from "./appearance/AppearanceSection";
import { LlmSection } from "./llm/LlmSection";
import { SettingsSide } from "./SettingsSide";
import { SystemSection } from "./system/SystemSection";
import type { SettingsViewProps } from "./types";

export function SettingsView({
  activeTab,
  themeMode,
  onTabChange,
  onThemeModeChange,
  onBack,
}: SettingsViewProps) {
  const { t } = useTranslation();

  return (
    <section className={`settings settings--${activeTab}`}>
      <SettingsSide
        activeTab={activeTab}
        onTabChange={onTabChange}
        onBack={onBack}
      />
      <div className="settings-main">
        <div className="settings-panel" key={activeTab}>
          {activeTab !== "llm" && (
            <h1 className="settings-page-title">{t(TAB_TITLE_KEYS[activeTab])}</h1>
          )}
          {activeTab === "appearance" && (
            <AppearanceSection
              themeMode={themeMode}
              onThemeModeChange={onThemeModeChange}
            />
          )}
          {activeTab === "llm" && <LlmSection />}
          {activeTab === "system" && <SystemSection />}
        </div>
      </div>
    </section>
  );
}
