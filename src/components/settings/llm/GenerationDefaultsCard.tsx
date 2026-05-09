import { useTranslation } from "react-i18next";
import { ASPECT_RATIOS, IMAGE_SIZES } from "../../../config/generation";
import { useSettings } from "../../../store/settings";

export function GenerationDefaultsCard() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);

  return (
    <div className="settings-card">
      <div className="settings-card-title">{t("settings.llm.defaultsTitle")}</div>
      <div className="settings-card-desc">{t("settings.llm.defaultsDesc")}</div>

      <div className="row">
        <label className="field-label">{t("settings.llm.ratioLabel")}</label>
        <div className="chips ratio-grid">
          {ASPECT_RATIOS.map((ratio) => (
            <button
              key={ratio}
              type="button"
              className={`chip ${settings?.default_aspect_ratio === ratio ? "active" : ""}`}
              onClick={() => update({ default_aspect_ratio: ratio })}
            >
              {ratio === "auto" ? t("common.auto") : ratio}
            </button>
          ))}
        </div>
      </div>

      <div className="row">
        <label className="field-label">{t("settings.llm.sizeLabel")}</label>
        <div className="chips">
          {IMAGE_SIZES.map((size) => (
            <button
              key={size}
              type="button"
              className={`chip ${settings?.default_image_size === size ? "active" : ""}`}
              onClick={() => update({ default_image_size: size })}
            >
              {size === "auto" ? t("common.auto") : size}
            </button>
          ))}
        </div>
        <div className="hint">{t("settings.llm.sizeHint")}</div>
      </div>
    </div>
  );
}
