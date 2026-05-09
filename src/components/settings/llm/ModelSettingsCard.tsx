import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { MODEL_PRESETS } from "../../../config/generation";
import { useSettings } from "../../../store/settings";
import { CheckIcon } from "../icons";

export function ModelSettingsCard() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const [model, setModel] = useState("");

  useEffect(() => {
    if (!settings) return;
    setModel(settings.model);
  }, [settings]);

  const modelDirty = !!settings && model.trim() !== settings.model;

  const saveModel = async () => {
    const value = model.trim();
    if (!value || !modelDirty) return;
    await update({ model: value });
  };

  return (
    <div className="settings-card">
      <div className="settings-card-title">{t("settings.llm.modelTitle")}</div>
      <div className="settings-card-desc">{t("settings.llm.modelDesc")}</div>

      <div className="row">
        <label className="field-label">{t("settings.llm.modelIdLabel")}</label>
        <div className="settings-inline-row">
          <input
            type="text"
            value={model}
            placeholder={t("settings.llm.modelIdPlaceholder")}
            spellCheck={false}
            onChange={(e) => setModel(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                saveModel();
              }
            }}
          />
          <button
            type="button"
            className="btn"
            onClick={saveModel}
            disabled={!modelDirty || !model.trim()}
          >
            {t("settings.llm.modelApply")}
          </button>
        </div>
      </div>

      <div className="settings-preset-list">
        {MODEL_PRESETS.map((preset) => (
          <button
            key={preset}
            type="button"
            className={`settings-preset-item ${preset === settings?.model ? "active" : ""}`}
            onClick={() => {
              setModel(preset);
              if (preset !== settings?.model) update({ model: preset });
            }}
          >
            <span>{preset}</span>
            {preset === settings?.model && <CheckIcon />}
          </button>
        ))}
      </div>
    </div>
  );
}
