import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";

export function ApiSettingsCard() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);

  const [endpoint, setEndpoint] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [savingApi, setSavingApi] = useState(false);

  useEffect(() => {
    if (!settings) return;
    setEndpoint(settings.endpoint);
    setApiKey(settings.api_key);
  }, [settings]);

  const apiDirty =
    !!settings && (endpoint !== settings.endpoint || apiKey !== settings.api_key);

  const saveApi = async () => {
    if (!apiDirty) return;
    setSavingApi(true);
    try {
      await update({ endpoint, api_key: apiKey });
    } finally {
      setSavingApi(false);
    }
  };

  return (
    <div className="settings-card">
      <div className="settings-card-title">{t("settings.llm.apiTitle")}</div>
      <div className="settings-card-desc">{t("settings.llm.apiDesc")}</div>

      <div className="row">
        <label className="field-label">{t("settings.llm.endpointLabel")}</label>
        <input
          type="text"
          value={endpoint}
          placeholder={t("settings.llm.endpointPlaceholder")}
          spellCheck={false}
          onChange={(e) => setEndpoint(e.target.value)}
        />
      </div>

      <div className="row">
        <label className="field-label">{t("settings.llm.apiKeyLabel")}</label>
        <div className="input-affix">
          <input
            type={showKey ? "text" : "password"}
            value={apiKey}
            placeholder={t("settings.llm.apiKeyPlaceholder")}
            autoComplete="off"
            spellCheck={false}
            onChange={(e) => setApiKey(e.target.value)}
          />
          <button
            type="button"
            className="affix-btn"
            onClick={() => setShowKey((v) => !v)}
          >
            {showKey ? t("settings.llm.keyHide") : t("settings.llm.keyShow")}
          </button>
        </div>
        <div className="hint">{t("settings.llm.keyHint")}</div>
      </div>

      <div className="settings-card-actions">
        <button
          type="button"
          className="btn primary"
          onClick={saveApi}
          disabled={!apiDirty || savingApi}
        >
          {savingApi
            ? t("common.saving")
            : apiDirty
            ? t("common.save")
            : t("common.saved")}
        </button>
      </div>
    </div>
  );
}
