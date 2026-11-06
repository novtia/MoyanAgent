import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api/tauri";
import type {
  AgentDefinitionInfo,
  AgentSummary,
  CustomAgent,
} from "../../../types";
import { ToolGlyph } from "../../chat/messageList/toolIcons";
import { ProviderEnableSwitch } from "../llm/modelService/ProviderEnableSwitch";

const NEW_KEY = "__new__";

type Draft = {
  name: string;
  whenToUse: string;
  systemPrompt: string;
  model: string;
  tools: string[];
};

const EMPTY_DRAFT: Draft = {
  name: "",
  whenToUse: "",
  systemPrompt: "",
  model: "",
  tools: [],
};

export function AgentManager({
  toolEnabled,
  disabledAgents,
  onSetToolEnabled,
  onSetDisabledAgents,
}: {
  toolEnabled: boolean;
  disabledAgents: string[];
  onSetToolEnabled: (enabled: boolean) => void;
  onSetDisabledAgents: (next: string[]) => void;
}) {
  const { t } = useTranslation();
  const [agents, setAgents] = useState<AgentSummary[] | null>(null);
  const [customs, setCustoms] = useState<CustomAgent[]>([]);
  const [toolNames, setToolNames] = useState<string[]>([]);
  const [open, setOpen] = useState<string | null>(null);
  const [definition, setDefinition] = useState<AgentDefinitionInfo | null>(null);
  const [draft, setDraft] = useState<Draft>(EMPTY_DRAFT);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const reload = useCallback(async () => {
    try {
      const [a, c] = await Promise.all([api.listAgents(), api.listCustomAgents()]);
      setAgents(a);
      setCustoms(c);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void reload();
    api.listAgentTools().then(setToolNames).catch(() => undefined);
  }, [reload]);

  const disabled = new Set(disabledAgents);
  const customOf = (type: string) => customs.find((c) => c.agent_type === type);

  const toggleAgent = (type: string, enabled: boolean) => {
    const next = enabled
      ? disabledAgents.filter((x) => x !== type)
      : disabled.has(type)
        ? disabledAgents
        : [...disabledAgents, type];
    onSetDisabledAgents(next);
  };

  const openAgent = (type: string) => {
    if (open === type) {
      setOpen(null);
      return;
    }
    setError(null);
    setOpen(type);
    setDefinition(null);
    const c = customOf(type);
    if (c) {
      setDraft({
        name: c.name,
        whenToUse: c.when_to_use,
        systemPrompt: c.system_prompt,
        model: c.model ?? "",
        tools: c.tools,
      });
    } else {
      api
        .getAgentDefinition(type)
        .then(setDefinition)
        .catch((e) => setError(String(e)));
    }
  };

  const openNew = () => {
    setError(null);
    setDefinition(null);
    setDraft(EMPTY_DRAFT);
    setOpen(NEW_KEY);
  };

  const save = async () => {
    const name = draft.name.trim();
    if (!name) return;
    setBusy(true);
    setError(null);
    try {
      const payload = {
        name,
        whenToUse: draft.whenToUse,
        systemPrompt: draft.systemPrompt,
        model: draft.model.trim() || null,
        tools: draft.tools,
      };
      if (open === NEW_KEY) {
        const created = await api.createCustomAgent(payload);
        await reload();
        setOpen(created.agent_type);
      } else if (open) {
        await api.updateCustomAgent({ agentType: open, ...payload });
        await reload();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    if (!open || open === NEW_KEY) return;
    if (!window.confirm(t("settings.tools.agentMgr.confirmDelete"))) return;
    setBusy(true);
    try {
      await api.deleteCustomAgent(open);
      if (disabled.has(open)) {
        onSetDisabledAgents(disabledAgents.filter((x) => x !== open));
      }
      setOpen(null);
      await reload();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const toggleTool = (name: string) =>
    setDraft((d) => ({
      ...d,
      tools: d.tools.includes(name)
        ? d.tools.filter((x) => x !== name)
        : [...d.tools, name],
    }));

  const ordered = (agents ?? [])
    .slice()
    .sort((a, b) => Number(a.custom) - Number(b.custom));
  const descOf = (type: string, raw: string, custom: boolean) =>
    custom
      ? (customOf(type)?.when_to_use ?? raw)
      : t(`settings.tools.agentMgr.builtinDesc.${type}`, { defaultValue: raw });
  const customName = (a: AgentSummary) => customOf(a.agent_type)?.name ?? a.agent_type;

  const editor = (isNew: boolean) => (
    <div className="agent-mgr-editor">
      <label className="field-label">{t("settings.tools.agentMgr.name")}</label>
      <input
        className="field-input"
        value={draft.name}
        onChange={(e) => setDraft({ ...draft, name: e.target.value })}
      />
      <label className="field-label">{t("settings.tools.agentMgr.whenToUse")}</label>
      <textarea
        className="field-input"
        rows={2}
        value={draft.whenToUse}
        onChange={(e) => setDraft({ ...draft, whenToUse: e.target.value })}
      />
      <label className="field-label">{t("settings.tools.agentMgr.systemPrompt")}</label>
      <textarea
        className="field-input field-input--lg settings-tool-desc-editor"
        rows={8}
        spellCheck={false}
        value={draft.systemPrompt}
        onChange={(e) => setDraft({ ...draft, systemPrompt: e.target.value })}
      />
      <label className="field-label">{t("settings.tools.agentMgr.model")}</label>
      <input
        className="field-input"
        value={draft.model}
        onChange={(e) => setDraft({ ...draft, model: e.target.value })}
      />
      <label className="field-label">{t("settings.tools.agentMgr.tools")}</label>
      <div className="agent-mgr-chips">
        {toolNames.map((name) => (
          <button
            key={name}
            type="button"
            className={`agent-mgr-chip${draft.tools.includes(name) ? " is-on" : ""}`}
            onClick={() => toggleTool(name)}
          >
            {name}
          </button>
        ))}
        {draft.tools.length === 0 && (
          <span className="hint">{t("settings.tools.agentMgr.allTools")}</span>
        )}
      </div>
      <div className="agent-mgr-actions">
        <button
          type="button"
          className="btn btn-primary"
          disabled={busy || !draft.name.trim()}
          onClick={() => void save()}
        >
          {t("settings.tools.agentMgr.save")}
        </button>
        {!isNew && (
          <button
            type="button"
            className="btn"
            disabled={busy}
            onClick={() => void remove()}
          >
            {t("settings.tools.agentMgr.delete")}
          </button>
        )}
      </div>
    </div>
  );

  const readOnly = () => (
    <div className="agent-mgr-editor">
      {!definition ? (
        <span className="hint">{t("settings.tools.agentMgr.loading")}</span>
      ) : (
        <>
          <p className="hint">{t("settings.tools.agentMgr.readOnlyHint")}</p>
          <label className="field-label">{t("settings.tools.agentMgr.whenToUse")}</label>
          <p className="agent-mgr-text">
            {descOf(definition.agent_type, definition.when_to_use, false) || "-"}
          </p>
          <label className="field-label">{t("settings.tools.agentMgr.systemPrompt")}</label>
          <textarea
            className="field-input field-input--lg settings-tool-desc-editor"
            rows={8}
            readOnly
            value={definition.system_prompt || t("settings.tools.agentMgr.noPrompt")}
          />
          <label className="field-label">{t("settings.tools.agentMgr.tools")}</label>
          <div className="agent-mgr-chips">
            {definition.tools.map((x) => (
              <span key={x} className="agent-mgr-chip is-on">
                {x === "*" ? t("settings.tools.agentMgr.allTools") : x}
              </span>
            ))}
            {(definition.disallowed_tools ?? []).map((x) => (
              <span key={`d-${x}`} className="agent-mgr-chip is-denied">
                − {x}
              </span>
            ))}
          </div>
          {definition.background && (
            <p className="hint">{t("settings.tools.agentMgr.background")}</p>
          )}
        </>
      )}
    </div>
  );

  return (
    <section className="model-provider-detail">
      <div className="model-provider-detail-inner">
        <header className="model-provider-hero">
          <span className="model-provider-avatar settings-tool-avatar" data-tool="Agent">
            <ToolGlyph tool="Agent" />
          </span>
          <div className="model-provider-hero-text">
            <h2 className="model-provider-hero-name">
              {t("settings.tools.agentMgr.title")}
            </h2>
            <span className="model-provider-hero-sdk">Agent</span>
          </div>
          <div className="model-provider-title-tools">
            <ProviderEnableSwitch
              enabled={toolEnabled}
              onChange={onSetToolEnabled}
              title={
                toolEnabled
                  ? t("settings.tools.disableTool")
                  : t("settings.tools.enableTool")
              }
            />
          </div>
        </header>
        <p className="hint">{t("settings.tools.agentMgr.desc")}</p>

        <div className="model-list-head">
          <div className="model-list-title">
            {t("settings.tools.agentMgr.title")}
            <span>{ordered.length}</span>
          </div>
          <button type="button" className="btn" onClick={openNew}>
            + {t("settings.tools.agentMgr.add")}
          </button>
        </div>

        {error && <span className="hint is-error">{error}</span>}
        {!agents && !error && (
          <div className="model-empty-state">{t("settings.tools.agentMgr.loading")}</div>
        )}

        <div className="agent-mgr-list">
          {open === NEW_KEY && (
            <div className="agent-mgr-item is-open">
              <div className="agent-mgr-row">
                <span className="agent-mgr-name">
                  {t("settings.tools.agentMgr.add")}
                </span>
              </div>
              {editor(true)}
            </div>
          )}
          {ordered.map((a) => {
            const enabled = !disabled.has(a.agent_type);
            const isOpen = open === a.agent_type;
            return (
              <div
                key={a.agent_type}
                className={`agent-mgr-item${isOpen ? " is-open" : ""}${enabled ? "" : " is-disabled"}`}
              >
                <div className="agent-mgr-row">
                  <button
                    type="button"
                    className="agent-mgr-main"
                    onClick={() => openAgent(a.agent_type)}
                  >
                    <span className="agent-mgr-name">
                      {customName(a)}
                      <span className={`agent-mgr-tag${a.custom ? " is-custom" : ""}`}>
                        {a.custom
                          ? t("settings.tools.agentMgr.custom")
                          : t("settings.tools.agentMgr.builtin")}
                      </span>
                    </span>
                    {a.when_to_use && (
                      <span className="agent-mgr-desc">
                        {descOf(a.agent_type, a.when_to_use, a.custom)}
                      </span>
                    )}
                  </button>
                  <ProviderEnableSwitch
                    enabled={enabled}
                    onChange={(next) => toggleAgent(a.agent_type, next)}
                    title={
                      enabled
                        ? t("settings.tools.agentMgr.disable")
                        : t("settings.tools.agentMgr.enable")
                    }
                  />
                </div>
                {isOpen && (a.custom ? editor(false) : readOnly())}
              </div>
            );
          })}
        </div>
      </div>
    </section>
  );
}
