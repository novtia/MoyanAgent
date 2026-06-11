import { useTranslation } from "react-i18next";
import {
  CHAT_FONT_FAMILY_PRESETS,
  CHAT_FONT_SIZE_RANGE,
  CHAT_LINE_HEIGHT_RANGE,
  useChatFont,
} from "../../store/chatFont";

/** Fallback swatch shown in the color picker while the "default" (theme ink)
 * color is active and no explicit color has been chosen. */
const DEFAULT_COLOR_SWATCH = "#1a1a1a";

export function ChatFontPanel() {
  const { t } = useTranslation();
  const fontFamily = useChatFont((s) => s.fontFamily);
  const fontSize = useChatFont((s) => s.fontSize);
  const lineHeight = useChatFont((s) => s.lineHeight);
  const color = useChatFont((s) => s.color);
  const set = useChatFont((s) => s.set);
  const reset = useChatFont((s) => s.reset);

  const isDefaultColor = color === "default";
  const colorValue = isDefaultColor ? DEFAULT_COLOR_SWATCH : color;

  return (
    <div className="chat-font-panel" role="dialog" aria-label={t("chat.fontSettings")}>
      <div className="chat-font-panel-head">
        <span className="chat-font-panel-title">{t("chat.fontSettings")}</span>
        <button
          type="button"
          className="chat-font-reset"
          onClick={reset}
        >
          {t("chat.fontReset")}
        </button>
      </div>

      <label className="chat-font-row">
        <span className="chat-font-label">{t("chat.fontFamily")}</span>
        <select
          className="chat-font-select"
          value={fontFamily}
          onChange={(e) => set({ fontFamily: e.target.value })}
        >
          {CHAT_FONT_FAMILY_PRESETS.map((preset) => (
            <option key={preset.id} value={preset.value}>
              {t(`chat.${preset.labelKey}`)}
            </option>
          ))}
        </select>
      </label>

      <label className="chat-font-row">
        <span className="chat-font-label">
          {t("chat.fontSize")}
          <span className="chat-font-value">{fontSize}px</span>
        </span>
        <input
          type="range"
          className="app-slider"
          min={CHAT_FONT_SIZE_RANGE.min}
          max={CHAT_FONT_SIZE_RANGE.max}
          step={1}
          value={fontSize}
          onChange={(e) => set({ fontSize: Number(e.target.value) })}
        />
      </label>

      <label className="chat-font-row">
        <span className="chat-font-label">
          {t("chat.lineHeight")}
          <span className="chat-font-value">{lineHeight.toFixed(1)}</span>
        </span>
        <input
          type="range"
          className="app-slider"
          min={CHAT_LINE_HEIGHT_RANGE.min}
          max={CHAT_LINE_HEIGHT_RANGE.max}
          step={0.1}
          value={lineHeight}
          onChange={(e) => set({ lineHeight: Number(e.target.value) })}
        />
      </label>

      <div className="chat-font-row">
        <span className="chat-font-label">{t("chat.fontColor")}</span>
        <div className="chat-font-color">
          <input
            type="color"
            className="chat-font-color-input"
            value={colorValue}
            onChange={(e) => set({ color: e.target.value })}
          />
          <button
            type="button"
            className={`chat-font-color-default ${isDefaultColor ? "is-active" : ""}`}
            onClick={() => set({ color: "default" })}
            disabled={isDefaultColor}
          >
            {t("chat.fontColorDefault")}
          </button>
        </div>
      </div>
    </div>
  );
}
