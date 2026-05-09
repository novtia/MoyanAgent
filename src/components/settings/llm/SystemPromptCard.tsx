import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";

export function SystemPromptCard() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const [draft, setDraft] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    setDraft(settings?.system_prompt ?? "");
  }, [settings?.system_prompt]);

  const dirty = !!settings && draft !== (settings.system_prompt ?? "");

  const save = async () => {
    if (!dirty) return;
    setSaving(true);
    try {
      await update({ system_prompt: draft });
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="settings-card">
      <div className="settings-card-title">{t("settings.llm.systemPromptTitle")}</div>
      <div className="settings-card-desc">{t("settings.llm.systemPromptDesc")}</div>
      <div className="row">
        <textarea
          className="settings-system-prompt field-input field-input--lg"
          rows={4}
          value={draft}
          spellCheck={false}
          placeholder={t("settings.llm.systemPromptPlaceholder")}
          onChange={(e) => setDraft(e.target.value)}
        />
      </div>
      <div className="settings-card-actions">
        <button
          type="button"
          className="btn primary"
          onClick={save}
          disabled={!dirty || saving}
        >
          {saving ? t("common.saving") : dirty ? t("common.save") : t("common.saved")}
        </button>
      </div>
    </div>
  );
}
