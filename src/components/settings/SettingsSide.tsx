import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { ArrowLeftIcon, SparkIcon, SunIcon, TerminalIcon } from "./icons";
import type { SettingsTab } from "./types";

interface SettingsSideProps {
  activeTab: SettingsTab;
  onTabChange: (tab: SettingsTab) => void;
  onBack: () => void;
}

interface SettingsNavItemProps {
  icon: ReactNode;
  label: string;
  active?: boolean;
  onClick: () => void;
}

export function SettingsSide({
  activeTab,
  onTabChange,
  onBack,
}: SettingsSideProps) {
  const { t } = useTranslation();

  return (
    <aside className="settings-side">
      <button type="button" className="settings-back" onClick={onBack}>
        <ArrowLeftIcon />
        <span>{t("settings.backToApp")}</span>
      </button>

      <nav className="settings-nav">
        <SettingsNavItem
          icon={<SunIcon />}
          label={t("settings.tabAppearance")}
          active={activeTab === "appearance"}
          onClick={() => onTabChange("appearance")}
        />
        <SettingsNavItem
          icon={<SparkIcon />}
          label={t("settings.tabLlm")}
          active={activeTab === "llm"}
          onClick={() => onTabChange("llm")}
        />
        <SettingsNavItem
          icon={<TerminalIcon />}
          label={t("settings.tabSystem")}
          active={activeTab === "system"}
          onClick={() => onTabChange("system")}
        />
      </nav>
    </aside>
  );
}

function SettingsNavItem({
  icon,
  label,
  active,
  onClick,
}: SettingsNavItemProps) {
  return (
    <button
      type="button"
      className={`settings-nav-item ${active ? "active" : ""}`}
      onClick={onClick}
    >
      {icon}
      <span>{label}</span>
    </button>
  );
}
