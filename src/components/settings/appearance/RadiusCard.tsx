import { useTranslation } from "react-i18next";
import { SegmentButton } from "../SegmentButton";
import { useAppearance, type RadiusOption } from "../../../store/appearance";

const RADIUS_OPTIONS: RadiusOption[] = ["sharp", "default", "rounded"];

export function RadiusCard() {
  const { t } = useTranslation();
  const radius = useAppearance((s) => s.radius);
  const set = useAppearance((s) => s.set);

  return (
    <div className="settings-row">
      <div className="settings-row-main">
        <div className="settings-row-title">
          {t("settings.appearance.radiusTitle")}
        </div>
        <div className="settings-row-desc">
          {t("settings.appearance.radiusDesc")}
        </div>
      </div>
      <div className="settings-row-control">
        <div className="settings-segment" role="tablist">
          {RADIUS_OPTIONS.map((option) => (
            <SegmentButton
              key={option}
              active={radius === option}
              onClick={() => set({ radius: option })}
            >
              <span className={`appearance-radius-preview appearance-radius-preview--${option}`} />
              <span>{t(`settings.appearance.radius.${option}`)}</span>
            </SegmentButton>
          ))}
        </div>
      </div>
    </div>
  );
}
