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
      <div className="settings-card-head settings-card-head--stack">
        <div>
          <div className="settings-card-title">
            {t("settings.appearance.typographyTitle")}
          </div>
          <div className="settings-card-desc">
            {t("settings.appearance.typographyDesc")}
          </div>
        </div>

        <label className="appearance-field-row">
          <span className="appearance-field-label">
            {t("settings.appearance.uiFontLabel")}
          </span>
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
        </label>

        <div className="appearance-chat-font">
          <div className="appearance-subsection-title">
            {t("settings.appearance.chatFontTitle")}
          </div>
          <ChatFontControls
            className="appearance-chat-font-controls"
            hideReset
          />
        </div>
      </div>
    </div>
  );
}
