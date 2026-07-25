import { useTranslation } from "react-i18next";
import { SegmentButton } from "../SegmentButton";
import {
  useAppearance,
  type ChatWidthOption,
  type DensityOption,
} from "../../../store/appearance";
import { RadiusCard } from "./RadiusCard";

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
      <div className="settings-row">
        <div className="settings-row-main">
          <div className="settings-row-title">
            {t("settings.appearance.chatWidthLabel")}
          </div>
          <div className="settings-row-desc">
            {t("settings.appearance.layoutDesc")}
          </div>
        </div>
        <div className="settings-row-control">
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
      </div>

      <div className="settings-row">
        <div className="settings-row-main">
          <div className="settings-row-title">
            {t("settings.appearance.densityLabel")}
          </div>
        </div>
        <div className="settings-row-control">
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

      <RadiusCard />
    </div>
  );
}
