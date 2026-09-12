import { useTranslation } from "react-i18next";
import { ToolGlyph } from "../../chat/messageList/toolIcons";
import { toolDescription } from "../../chat/rightPanel/agentFlow/toolUtils";
import { ProviderEnableSwitch } from "../llm/modelService/ProviderEnableSwitch";
import type { AgentToolSpec } from "../../../types";
import { schemaFields } from "./schemaFields";

export function ToolDetail({
  selected,
  enabled,
  echoContent,
  replaceAllDefault,
  paragraphLabels,
  onSetEnabled,
  onSetEchoContent,
  onSetReplaceAllDefault,
  onSetParagraphLabels,
}: {
  selected: AgentToolSpec | null;
  enabled: boolean;
  echoContent: boolean;
  replaceAllDefault: boolean;
  paragraphLabels: boolean;
  onSetEnabled: (name: string, enabled: boolean) => void;
  onSetEchoContent: (enabled: boolean) => void;
  onSetReplaceAllDefault: (enabled: boolean) => void;
  onSetParagraphLabels: (enabled: boolean) => void;
}) {
  const { t } = useTranslation();

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

  const blurb = toolDescription(t, selected.name);
  const description = blurb === selected.name ? selected.description : blurb;
  const fields = schemaFields(selected.schema);

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

        <p className="settings-tool-detail-desc">{description}</p>

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
