import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { getLanguage, setLanguage } from "../../../i18n";
import { useAppearance } from "../../../store/appearance";
import { useChatFont } from "../../../store/chatFont";
import { DisplayIcon, MoonIcon, SunIcon } from "../icons";
import type { ThemeMode } from "../types";
import { AccentColorCard } from "./AccentColorCard";
import { LanguageDropdown } from "./LanguageDropdown";
import { LayoutCard } from "./LayoutCard";
import { TypographyCard } from "./TypographyCard";

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
  const resetAppearance = useAppearance((s) => s.reset);
  const resetChatFont = useChatFont((s) => s.reset);

  const handleResetAll = () => {
    resetAppearance();
    resetChatFont();
  };

  return (
    <>
      <div className="settings-card">
        <div className="settings-block">
          <div className="settings-block-head">
            <div className="settings-row-title">
              {t("settings.appearance.themeTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.appearance.themeDesc")}
            </div>
          </div>
          <div className="theme-grid" role="radiogroup">
            <ThemeTile
              mode="light"
              active={themeMode === "light"}
              icon={<SunIcon />}
              label={t("settings.appearance.light")}
              onSelect={() => onThemeModeChange("light")}
            />
            <ThemeTile
              mode="dark"
              active={themeMode === "dark"}
              icon={<MoonIcon />}
              label={t("settings.appearance.dark")}
              onSelect={() => onThemeModeChange("dark")}
            />
            <ThemeTile
              mode="system"
              active={themeMode === "system"}
              icon={<DisplayIcon />}
              label={t("settings.appearance.system")}
              onSelect={() => onThemeModeChange("system")}
            />
          </div>
        </div>
        <AccentColorCard />
      </div>

      <TypographyCard />
      <LayoutCard />

      <div className="settings-card">
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.appearance.languageTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.appearance.languageDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <LanguageDropdown
              current={currentLang}
              onChange={(lang) => setLanguage(lang)}
              ariaLabel={t("settings.appearance.languageTitle")}
            />
          </div>
        </div>
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">
              {t("settings.appearance.resetTitle")}
            </div>
            <div className="settings-row-desc">
              {t("settings.appearance.resetDesc")}
            </div>
          </div>
          <div className="settings-row-control">
            <button
              type="button"
              className="appearance-reset-btn"
              onClick={handleResetAll}
            >
              {t("settings.appearance.resetAction")}
            </button>
          </div>
        </div>
      </div>
    </>
  );
}

interface ThemeTileProps {
  mode: ThemeMode;
  active: boolean;
  icon: ReactNode;
  label: string;
  onSelect: () => void;
}

function ThemeTile({ mode, active, icon, label, onSelect }: ThemeTileProps) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      className={`theme-tile ${active ? "active" : ""}`}
      onClick={onSelect}
    >
      {mode === "system" ? (
        <span className="theme-preview theme-preview--system">
          <span className="tp-half tp-half--light">
            <MiniWindow />
          </span>
          <span className="tp-half tp-half--dark">
            <MiniWindow />
          </span>
        </span>
      ) : (
        <span className={`theme-preview theme-preview--${mode}`}>
          <MiniWindow />
        </span>
      )}
      <span className="theme-tile-label">
        {icon}
        <span>{label}</span>
      </span>
    </button>
  );
}

function MiniWindow() {
  return (
    <>
      <span className="tp-bar">
        <i />
        <i />
        <i />
      </span>
      <span className="tp-body">
        <span className="tp-line tp-w70" />
        <span className="tp-line tp-w45" />
        <span className="tp-pill" />
      </span>
    </>
  );
}
