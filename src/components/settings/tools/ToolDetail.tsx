import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { ToolGlyph } from "../../chat/messageList/toolIcons";
import {
  isCustomToolDescription,
  modelToolDescription,
} from "../../chat/rightPanel/agentFlow/toolUtils";
import { ProviderEnableSwitch } from "../llm/modelService/ProviderEnableSwitch";
import type { AgentToolSpec, NovelAiSettings } from "../../../types";
import { NovelAiConfigFields } from "./NovelAiConfigFields";
import { schemaFields } from "./schemaFields";

export function ToolDetail({
  selected,
  enabled,
  forced,
  echoContent,
  replaceAllDefault,
  paragraphLabels,
  novelai,
  descriptions,
  onSetEnabled,
  onSetForced,
  onSetEchoContent,
  onSetReplaceAllDefault,
  onSetParagraphLabels,
  onSetNovelai,
  onSetDescription,
}: {
  selected: AgentToolSpec | null;
  enabled: boolean;
  forced: boolean;
  echoContent: boolean;
  replaceAllDefault: boolean;
  paragraphLabels: boolean;
  novelai: NovelAiSettings | undefined;
  descriptions: Record<string, string>;
  onSetEnabled: (name: string, enabled: boolean) => void;
  onSetForced: (name: string, forced: boolean) => void;
  onSetEchoContent: (enabled: boolean) => void;
  onSetReplaceAllDefault: (enabled: boolean) => void;
  onSetParagraphLabels: (enabled: boolean) => void;
  onSetNovelai: (next: NovelAiSettings) => void;
  onSetDescription: (name: string, text: string | null) => void;
}) {
  const { t } = useTranslation();
  const selectedName = selected?.name;
  const builtinDesc = selected?.description ?? "";
  const overrideText = selectedName ? (descriptions[selectedName] ?? "") : "";
  const [draft, setDraft] = useState(() =>
    selected ? modelToolDescription(selected, descriptions) : "",
  );

  useEffect(() => {
    if (!selectedName) {
      setDraft("");
      return;
    }
    setDraft(
      modelToolDescription(
        { name: selectedName, description: builtinDesc },
        { [selectedName]: overrideText },
      ),
    );
  }, [selectedName, builtinDesc, overrideText]);

  if (!selected) {
    return (
      <section className="model-provider-detail">
        <div className="model-provider-detail-inner">
          <div className="model-empty-state model-empty-state--center">
            {t("settings.tools.selectHint")}
          </div>
        </div>
      </section>
    );
  }

  const customized =
    isCustomToolDescription(selected, descriptions) ||
    draft.replace(/\r\n/g, "\n").trim() !==
      selected.description.replace(/\r\n/g, "\n").trim();
  const fields = schemaFields(selected.schema);

  const commitDescription = () => {
    const next = draft.replace(/\r\n/g, "\n").trim();
    const builtin = selected.description.replace(/\r\n/g, "\n").trim();
    const current = (descriptions[selected.name] ?? "").replace(/\r\n/g, "\n").trim();
    if (next === "" || next === builtin) {
      if (current !== "") onSetDescription(selected.name, null);
      setDraft(selected.description);
      return;
    }
    if (next !== current) onSetDescription(selected.name, next);
  };

  const resetDescription = () => {
    setDraft(selected.description);
    onSetDescription(selected.name, null);
  };

  return (
    <section className="model-provider-detail">
      <div className="model-provider-detail-inner">
        <header className="model-provider-hero">
          <span className="model-provider-avatar settings-tool-avatar">
            <ToolGlyph tool={selected.name} />
          </span>
          <div className="model-provider-hero-text">
            <h2 className="model-provider-hero-name">{selected.name}</h2>
            {selected.read_only && (
              <span className="model-provider-hero-sdk">
                {t("settings.tools.readOnly")}
              </span>
            )}
            {customized && (
              <span className="model-provider-hero-sdk">
                {t("settings.tools.descCustom")}
              </span>
            )}
            {forced && (
              <span className="model-provider-hero-sdk">
                {t("settings.tools.forceBadge")}
              </span>
            )}
          </div>
          <div className="model-provider-title-tools">
            <ProviderEnableSwitch
              enabled={enabled}
              onChange={(next) => onSetEnabled(selected.name, next)}
              title={
                enabled
                  ? t("settings.tools.disableTool")
                  : t("settings.tools.enableTool")
              }
            />
          </div>
        </header>

        <div className="model-provider-config">
          <div className="settings-tool-desc-head">
            <span className="field-label">{t("settings.tools.descTitle")}</span>
            {customized && (
              <button
                type="button"
                className="appearance-reset-btn"
                onClick={resetDescription}
              >
                {t("settings.tools.descReset")}
              </button>
            )}
          </div>
          <p className="hint">{t("settings.tools.descDesc")}</p>
          <textarea
            className="field-input field-input--lg settings-tool-desc-editor"
            value={draft}
            rows={8}
            spellCheck={false}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commitDescription}
          />
        </div>

        <div className="model-provider-config">
          <div className="model-provider-switch-row">
            <div className="model-provider-switch-head">
              <span className="field-label">{t("settings.tools.enableTitle")}</span>
              <ProviderEnableSwitch
                enabled={enabled}
                onChange={(next) => onSetEnabled(selected.name, next)}
              />
            </div>
            <p className="hint">{t("settings.tools.enableDesc")}</p>
          </div>
        </div>

        <div className="model-provider-config">
          <div className="model-provider-switch-row">
            <div className="model-provider-switch-head">
              <span className="field-label">{t("settings.tools.forceTitle")}</span>
              <ProviderEnableSwitch
                enabled={forced}
                onChange={(next) => onSetForced(selected.name, next)}
              />
            </div>
            <p className="hint">{t("settings.tools.forceDesc")}</p>
          </div>
        </div>

        {selected.name === "CreateDoc" && (
          <div className="model-provider-config">
            <div className="model-provider-switch-row">
              <div className="model-provider-switch-head">
                <span className="field-label">{t("settings.tools.echoTitle")}</span>
                <ProviderEnableSwitch
                  enabled={echoContent}
                  onChange={onSetEchoContent}
                />
              </div>
              <p className="hint">{t("settings.tools.echoDesc")}</p>
            </div>
          </div>
        )}

        {selected.name === "Read" && (
          <div className="model-provider-config">
            <div className="model-provider-switch-row">
              <div className="model-provider-switch-head">
                <span className="field-label">
                  {t("settings.tools.paragraphLabelsTitle")}
                </span>
                <ProviderEnableSwitch
                  enabled={paragraphLabels}
                  onChange={onSetParagraphLabels}
                />
              </div>
              <p className="hint">{t("settings.tools.paragraphLabelsDesc")}</p>
            </div>
          </div>
        )}

        {selected.name === "NovelAI" && (
          <NovelAiConfigFields value={novelai} onChange={onSetNovelai} />
        )}

        <div className="model-list-head">
          <div>
            <div className="model-list-title">
              {t("settings.tools.paramsTitle")}
              <span>{fields.length}</span>
            </div>
            <p className="model-list-desc">{t("settings.tools.paramsDesc")}</p>
          </div>
        </div>

        {fields.length === 0 ? (
          <div className="model-empty-state">{t("settings.tools.noParams")}</div>
        ) : (
          <div className="settings-tool-params">
            {fields.map((field) => (
              <div key={field.name} className="settings-tool-param">
                <div className="settings-tool-param-head">
                  <code className="settings-tool-param-name">{field.name}</code>
                  <span className="settings-tool-param-type">{field.type}</span>
                  <span
                    className={`settings-tool-param-req ${
                      field.required ? "is-required" : ""
                    }`}
                  >
                    {field.required
                      ? t("settings.tools.required")
                      : t("settings.tools.optional")}
                  </span>
                </div>
                {field.description && (
                  <p className="settings-tool-param-desc">{field.description}</p>
                )}
                {selected.name === "Edit" && field.name === "replace_all" && (
                  <div className="settings-tool-param-default">
                    <div className="settings-tool-param-default-row">
                      <span className="field-label">
                        {t("settings.tools.replaceAllDefaultTitle")}
                      </span>
                      <ProviderEnableSwitch
                        enabled={replaceAllDefault}
                        onChange={onSetReplaceAllDefault}
                        title={t("settings.tools.replaceAllDefaultTitle")}
                      />
                    </div>
                    <p className="hint">{t("settings.tools.replaceAllDefaultDesc")}</p>
                  </div>
                )}
              </div>
            ))}
          </div>
        )}
      </div>
    </section>
  );
}
