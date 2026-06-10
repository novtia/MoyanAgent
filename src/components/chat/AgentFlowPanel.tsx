import { useCallback, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { dialog } from "../ui";
import { useSession } from "../../store/session";
import { AgentFlowCanvas, MAIN } from "./AgentFlowCanvas";
import type { AgentSummary, CustomAgent } from "../../types";

const CUSTOM_PREFIX = "custom:";

interface FormState {
  mode: "closed" | "new" | "edit";
  agentType: string;
  name: string;
  whenToUse: string;
  systemPrompt: string;
  model: string;
  tools: string[];
}

const EMPTY_FORM: FormState = {
  mode: "closed",
  agentType: "",
  name: "",
  whenToUse: "",
  systemPrompt: "",
  model: "",
  tools: [],
};

/**
 * Right-panel tab: a free-form node canvas for arranging a session's agent
 * flow. The main agent is a fixed node; other built-in / custom agents are
 * dragged around it and wired into a sequence. The connection order is
 * flattened into the linear `agent_chain` persisted on the session.
 */
export function AgentFlowPanel({ open }: { open: boolean }) {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const setAgentChain = useSession((s) => s.setAgentChain);

  const sessionId = active?.session.id ?? null;
  const projectId = active?.session.project_id ?? null;
  // Sessions in a project share one agent flow record, so the canvas keys its
  // graph (and layout) by the project; standalone chats key by session.
  const flowScopeId = projectId ?? sessionId;
  const chain = useMemo(() => active?.session.agent_chain ?? [], [active]);

  const [builtins, setBuiltins] = useState<AgentSummary[]>([]);
  const [customs, setCustoms] = useState<CustomAgent[]>([]);
  const [allTools, setAllTools] = useState<string[]>([]);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);

  const refreshAgents = useCallback(async () => {
    try {
      const [b, c] = await Promise.all([api.listAgents(), api.listCustomAgents()]);
      setBuiltins(b);
      setCustoms(c);
    } catch (e) {
      console.warn(e);
    }
  }, []);

  useEffect(() => {
    void refreshAgents();
  }, [refreshAgents]);

  useEffect(() => {
    api
      .listAgentTools()
      .then(setAllTools)
      .catch((e) => console.warn(e));
  }, []);

  const builtinName = useCallback(
    (agentType: string) => t(`agentFlow.builtinNames.${agentType}`, { defaultValue: agentType }),
    [t],
  );

  const nameOf = useCallback(
    (agentType: string) => {
      if (agentType === MAIN) return t("agentFlow.mainName");
      if (agentType.startsWith(CUSTOM_PREFIX)) {
        const found = customs.find((c) => c.agent_type === agentType);
        return found?.name ?? agentType.slice(CUSTOM_PREFIX.length);
      }
      return builtinName(agentType);
    },
    [customs, t, builtinName],
  );

  const agents = useMemo(
    () => [
      ...builtins.map((b) => ({ id: b.agent_type, name: builtinName(b.agent_type), custom: false })),
      ...customs.map((c) => ({ id: c.agent_type, name: c.name, custom: true })),
    ],
    [builtins, customs, builtinName],
  );

  const knownTypes = useMemo(() => {
    const s = new Set<string>();
    for (const b of builtins) s.add(b.agent_type);
    for (const c of customs) s.add(c.agent_type);
    return s;
  }, [builtins, customs]);

  const onOrderChange = useCallback(
    (order: string[]) => {
      if (!sessionId) return;
      const onlyMain = order.length === 1 && order[0] === MAIN;
      void setAgentChain(onlyMain ? [] : order);
    },
    [sessionId, setAgentChain],
  );

  // New agents start with every available tool enabled (full access); the user
  // can then uncheck tools to restrict the agent.
  const openNew = () => setForm({ ...EMPTY_FORM, mode: "new", tools: [...allTools] });
  const openEditByType = useCallback(
    (agentType: string) => {
      const c = customs.find((x) => x.agent_type === agentType);
      if (!c) return;
      // An empty stored list means "all tools"; reflect that as everything checked.
      const tools = c.tools.length > 0 ? c.tools : [...allTools];
      setForm({
        mode: "edit",
        agentType: c.agent_type,
        name: c.name,
        whenToUse: c.when_to_use,
        systemPrompt: c.system_prompt,
        model: c.model ?? "",
        tools,
      });
    },
    [customs, allTools],
  );
  const closeForm = () => setForm(EMPTY_FORM);

  const toggleTool = useCallback((tool: string) => {
    setForm((f) => {
      const has = f.tools.includes(tool);
      return {
        ...f,
        tools: has ? f.tools.filter((t) => t !== tool) : [...f.tools, tool],
      };
    });
  }, []);

  const allToolsSelected = allTools.length > 0 && form.tools.length === allTools.length;
  const toggleAllTools = useCallback(() => {
    setForm((f) => ({
      ...f,
      tools: f.tools.length === allTools.length ? [] : [...allTools],
    }));
  }, [allTools]);

  const submitForm = async () => {
    const name = form.name.trim();
    if (!name) return;
    // When every tool is selected, persist an empty list so the agent keeps
    // full access and automatically picks up tools added later.
    const tools = allToolsSelected ? [] : form.tools;
    try {
      if (form.mode === "new") {
        await api.createCustomAgent({
          name,
          whenToUse: form.whenToUse,
          systemPrompt: form.systemPrompt,
          model: form.model.trim() || null,
          tools,
        });
      } else if (form.mode === "edit") {
        await api.updateCustomAgent({
          agentType: form.agentType,
          name,
          whenToUse: form.whenToUse,
          systemPrompt: form.systemPrompt,
          model: form.model.trim() || null,
          tools,
        });
      }
      await refreshAgents();
      closeForm();
    } catch (e) {
      console.warn(e);
    }
  };

  const deleteByType = useCallback(
    async (agentType: string) => {
      const c = customs.find((x) => x.agent_type === agentType);
      if (!c) return;
      const ok = await dialog.confirm(t("agentFlow.deleteAgentConfirm", { name: c.name }), {
        type: "danger",
        confirmLabel: t("agentFlow.deleteAgent"),
        title: t("agentFlow.deleteAgent"),
      });
      if (!ok) return;
      try {
        await api.deleteCustomAgent(c.agent_type);
        await refreshAgents();
      } catch (e) {
        console.warn(e);
      }
    },
    [customs, refreshAgents, t],
  );

  return (
    <>
      <AgentFlowCanvas
        open={open}
        sessionId={sessionId}
        scopeId={flowScopeId}
        chain={chain}
        agents={agents}
        knownTypes={knownTypes}
        nameOf={nameOf}
        onOrderChange={onOrderChange}
        onRequestNewAgent={openNew}
        onEditAgent={openEditByType}
        onDeleteAgent={deleteByType}
      />
      {form.mode !== "closed" && (
        <div
          className="modal-backdrop"
          onMouseDown={(e) => {
            if (e.target === e.currentTarget) closeForm();
          }}
          onKeyDown={(e) => {
            if (e.key === "Escape") closeForm();
          }}
        >
          <div className="modal agent-flow-modal" role="dialog" aria-modal="true">
            <div className="modal-head">
              <h3>{form.mode === "new" ? t("agentFlow.newAgent") : t("agentFlow.editAgent")}</h3>
              <button type="button" className="close" onClick={closeForm}>
                {t("agentFlow.cancel")}
              </button>
            </div>
            <div className="modal-body">
              <div className="agent-flow-form agent-flow-form--modal">
                <label className="agent-flow-field">
                  <span>{t("agentFlow.formName")}</span>
                  <input
                    type="text"
                    value={form.name}
                    autoFocus
                    placeholder={t("agentFlow.formNamePlaceholder")}
                    onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
                  />
                </label>
                <label className="agent-flow-field">
                  <span>{t("agentFlow.formWhenToUse")}</span>
                  <input
                    type="text"
                    value={form.whenToUse}
                    placeholder={t("agentFlow.formWhenToUsePlaceholder")}
                    onChange={(e) => setForm((f) => ({ ...f, whenToUse: e.target.value }))}
                  />
                </label>
                <label className="agent-flow-field">
                  <span>{t("agentFlow.formSystemPrompt")}</span>
                  <textarea
                    rows={6}
                    value={form.systemPrompt}
                    placeholder={t("agentFlow.formSystemPromptPlaceholder")}
                    onChange={(e) => setForm((f) => ({ ...f, systemPrompt: e.target.value }))}
                  />
                </label>
                <label className="agent-flow-field">
                  <span>{t("agentFlow.formModel")}</span>
                  <input
                    type="text"
                    value={form.model}
                    placeholder={t("agentFlow.formModelPlaceholder")}
                    onChange={(e) => setForm((f) => ({ ...f, model: e.target.value }))}
                  />
                </label>
                <div className="agent-flow-field">
                  <div className="agent-flow-tools-head">
                    <span>{t("agentFlow.formTools")}</span>
                    <button
                      type="button"
                      className="agent-flow-tools-toggle"
                      onClick={toggleAllTools}
                      disabled={allTools.length === 0}
                    >
                      {allToolsSelected
                        ? t("agentFlow.toolsDeselectAll")
                        : t("agentFlow.toolsSelectAll")}
                    </button>
                  </div>
                  {allTools.length === 0 ? (
                    <p className="agent-flow-tools-empty">{t("agentFlow.toolsEmpty")}</p>
                  ) : (
                    <div className="agent-flow-tools">
                      {allTools.map((tool) => (
                        <label key={tool} className="agent-flow-tool">
                          <input
                            type="checkbox"
                            checked={form.tools.includes(tool)}
                            onChange={() => toggleTool(tool)}
                          />
                          <span>{tool}</span>
                        </label>
                      ))}
                    </div>
                  )}
                  <p className="agent-flow-tools-hint">{t("agentFlow.formToolsHint")}</p>
                </div>
              </div>
            </div>
            <div className="modal-foot">
              <button type="button" className="btn" onClick={closeForm}>
                {t("agentFlow.cancel")}
              </button>
              <button
                type="button"
                className="btn primary"
                disabled={!form.name.trim()}
                onClick={() => void submitForm()}
              >
                {form.mode === "new" ? t("agentFlow.create") : t("agentFlow.save")}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
