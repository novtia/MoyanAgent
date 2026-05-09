import { useTranslation } from "react-i18next";
import { getLanguage, setLanguage } from "../../../i18n";
import { DisplayIcon, MoonIcon, SunIcon } from "../icons";
import { SegmentButton } from "../SegmentButton";
import type { ThemeMode } from "../types";
import { LanguageDropdown } from "./LanguageDropdown";

interface AppearanceSectionProps {
  themeMode: ThemeMode;
  onThemeModeChange: (mode: ThemeMode) => void;
}

export function AppearanceSection({
  themeMode,
  onThemeModeChange,
}: AppearanceSectionProps) {
  const { t } = useTranslation();
  const currentLang = getLanguage();

  return (
    <>
      <div className="settings-card">
        <div className="settings-card-head">
          <div>
            <div className="settings-card-title">{t("settings.appearance.themeTitle")}</div>
            <div className="settings-card-desc">{t("settings.appearance.themeDesc")}</div>
          </div>
          <div className="settings-segment" role="tablist">
            <SegmentButton
              active={themeMode === "light"}
              onClick={() => onThemeModeChange("light")}
            >
              <SunIcon />
              <span>{t("settings.appearance.light")}</span>
            </SegmentButton>
            <SegmentButton
              active={themeMode === "dark"}
              onClick={() => onThemeModeChange("dark")}
            >
              <MoonIcon />
              <span>{t("settings.appearance.dark")}</span>
            </SegmentButton>
            <SegmentButton
              active={themeMode === "system"}
              onClick={() => onThemeModeChange("system")}
            >
              <DisplayIcon />
              <span>{t("settings.appearance.system")}</span>
            </SegmentButton>
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-card-head">
          <div>
            <div className="settings-card-title">{t("settings.appearance.languageTitle")}</div>
            <div className="settings-card-desc">{t("settings.appearance.languageDesc")}</div>
          </div>
          <LanguageDropdown
            current={currentLang}
            onChange={(lang) => setLanguage(lang)}
            ariaLabel={t("settings.appearance.languageTitle")}
          />
        </div>
      </div>
    </>
  );
}
