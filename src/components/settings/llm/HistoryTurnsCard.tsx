import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";

export function HistoryTurnsCard() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const current = settings?.history_turns ?? 10;
  const [draft, setDraft] = useState<string>(String(current));
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    setDraft(String(current));
    setError(null);
  }, [current]);

  const commit = () => {
    const trimmed = draft.trim();
    if (trimmed === "") {
      setError(t("settings.llm.paramInvalid"));
      return;
    }
    const parsed = Number.parseInt(trimmed, 10);
    if (!Number.isFinite(parsed) || parsed < 0 || parsed > 200) {
      setError(t("settings.llm.paramInvalid"));
      return;
    }
    setError(null);
    if (parsed !== current) update({ history_turns: parsed });
  };

  return (
    <div className="settings-card">
      <div className="settings-card-title">{t("settings.llm.historyTitle")}</div>
      <div className="settings-card-desc">{t("settings.llm.historyDesc")}</div>
      <div className="row">
        <label className="field-label">{t("settings.llm.historyTurnsLabel")}</label>
        <input
          type="number"
          className="field-input field-input--mono"
          inputMode="numeric"
          step="1"
          min={0}
          max={200}
          value={draft}
          placeholder="10"
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              commit();
            }
          }}
        />
        <div className={`hint ${error ? "is-error" : ""}`}>
          {error ?? t("settings.llm.historyTurnsHint")}
        </div>
      </div>
    </div>
  );
}
