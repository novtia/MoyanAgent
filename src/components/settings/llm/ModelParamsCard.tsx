import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";
import type { SettingsPatch } from "../../../types";
import { NUMERIC_FIELDS, type NumericFieldDef } from "./modelParams";
import { NumericParamField } from "./NumericParamField";

function numericPatch(
  key: NumericFieldDef["key"],
  next: number | null,
): SettingsPatch {
  return { [key]: next } as SettingsPatch;
}

export function ModelParamsCard() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);

  return (
    <div className="settings-card">
      <div className="settings-card-title">{t("settings.llm.paramsTitle")}</div>
      <div className="settings-card-desc">{t("settings.llm.paramsDesc")}</div>
      <div className="settings-params-grid">
        {NUMERIC_FIELDS.map((field) => (
          <NumericParamField
            key={field.key}
            def={field}
            value={settings ? settings[field.key] : null}
            onCommit={(next) => update(numericPatch(field.key, next))}
            invalidLabel={t("settings.llm.paramInvalid")}
            label={t(field.labelKey)}
            hint={t(field.hintKey)}
          />
        ))}
      </div>
    </div>
  );
}
