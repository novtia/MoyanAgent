import { useTranslation } from "react-i18next";
import {
  ACCENT_PRESETS,
  getAccentSwatchColor,
  useAppearance,
} from "../../../store/appearance";

const CUSTOM_FALLBACK = "#2563eb";

export function AccentColorCard() {
  const { t } = useTranslation();
  const accent = useAppearance((s) => s.accent);
  const set = useAppearance((s) => s.set);

  const isCustom = accent.startsWith("#");
  const customValue = getAccentSwatchColor(accent) ?? CUSTOM_FALLBACK;

  return (
    <div className="settings-card">
      <div className="settings-card-head settings-card-head--stack">
        <div>
          <div className="settings-card-title">
            {t("settings.appearance.accentTitle")}
          </div>
          <div className="settings-card-desc">
            {t("settings.appearance.accentDesc")}
          </div>
        </div>
        <div className="appearance-swatches" role="listbox" aria-label={t("settings.appearance.accentTitle")}>
          {ACCENT_PRESETS.map((preset) => {
            const active = accent === preset.id;
            const swatch =
              preset.id === "default"
                ? "var(--ink)"
                : (preset.color ?? CUSTOM_FALLBACK);
            return (
              <button
                key={preset.id}
                type="button"
                role="option"
                aria-selected={active}
                className={`appearance-swatch ${active ? "is-active" : ""} ${preset.id === "default" ? "appearance-swatch--default" : ""}`}
                style={
                  preset.id === "default"
                    ? undefined
                    : { background: swatch }
                }
                title={t(`settings.appearance.accent.${preset.id}`)}
                onClick={() => set({ accent: preset.id })}
              >
                {preset.id === "default" ? (
                  <span className="appearance-swatch-default-mark" />
                ) : null}
              </button>
            );
          })}
          <label
            className={`appearance-swatch appearance-swatch--custom ${isCustom ? "is-active" : ""}`}
            title={t("settings.appearance.accent.custom")}
          >
            <input
              type="color"
              className="appearance-swatch-color-input"
              value={customValue}
              onChange={(e) => set({ accent: e.target.value.toLowerCase() })}
              aria-label={t("settings.appearance.accent.custom")}
            />
          </label>
        </div>
      </div>
    </div>
  );
}
