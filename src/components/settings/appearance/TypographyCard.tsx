import { useTranslation } from "react-i18next";
import { ChatFontControls } from "../../chat/ChatFontControls";
import {
  UI_FONT_PRESETS,
  useAppearance,
  type UiFontOption,
} from "../../../store/appearance";

export function TypographyCard() {
  const { t } = useTranslation();
  const uiFont = useAppearance((s) => s.uiFont);
  const set = useAppearance((s) => s.set);

  return (
    <div className="settings-card">
      <div className="settings-row">
        <div className="settings-row-main">
          <div className="settings-row-title">
            {t("settings.appearance.uiFontLabel")}
          </div>
        </div>
        <div className="settings-row-control">
          <select
            className="chat-font-select appearance-select"
            value={uiFont}
            onChange={(e) => set({ uiFont: e.target.value as UiFontOption })}
          >
            {UI_FONT_PRESETS.map((preset) => (
              <option key={preset.id} value={preset.id}>
                {t(`settings.appearance.uiFont.${preset.id}`)}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="settings-row settings-row--stack">
        <div className="settings-row-main">
          <div className="settings-row-title">
            {t("settings.appearance.chatFontTitle")}
          </div>
          <div className="settings-row-desc">
            {t("settings.appearance.typographyDesc")}
          </div>
        </div>
        <ChatFontControls
          className="appearance-chat-font-controls"
          hideReset
        />
      </div>
    </div>
  );
}
