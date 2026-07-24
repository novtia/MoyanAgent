import { useTranslation } from "react-i18next";
import { SegmentButton } from "../SegmentButton";
import {
  useAppearance,
  type ChatWidthOption,
  type DensityOption,
} from "../../../store/appearance";

const WIDTH_OPTIONS: ChatWidthOption[] = ["narrow", "default", "wide", "full"];
const DENSITY_OPTIONS: DensityOption[] = [
  "compact",
  "comfortable",
  "spacious",
];

export function LayoutCard() {
  const { t } = useTranslation();
  const chatWidth = useAppearance((s) => s.chatWidth);
  const density = useAppearance((s) => s.density);
  const set = useAppearance((s) => s.set);

  return (
    <div className="settings-card">
      <div className="settings-card-head settings-card-head--stack">
        <div>
          <div className="settings-card-title">
            {t("settings.appearance.layoutTitle")}
          </div>
          <div className="settings-card-desc">
            {t("settings.appearance.layoutDesc")}
          </div>
        </div>

        <div className="appearance-control-block">
          <div className="appearance-field-label">
            {t("settings.appearance.chatWidthLabel")}
          </div>
          <div className="settings-segment" role="tablist">
            {WIDTH_OPTIONS.map((option) => (
              <SegmentButton
                key={option}
                active={chatWidth === option}
                onClick={() => set({ chatWidth: option })}
              >
                <span>{t(`settings.appearance.chatWidth.${option}`)}</span>
              </SegmentButton>
            ))}
          </div>
        </div>

        <div className="appearance-control-block">
          <div className="appearance-field-label">
            {t("settings.appearance.densityLabel")}
          </div>
          <div className="settings-segment" role="tablist">
            {DENSITY_OPTIONS.map((option) => (
              <SegmentButton
                key={option}
                active={density === option}
                onClick={() => set({ density: option })}
              >
                <span>{t(`settings.appearance.density.${option}`)}</span>
              </SegmentButton>
            ))}
          </div>
        </div>
      </div>
    </div>
  );
}
