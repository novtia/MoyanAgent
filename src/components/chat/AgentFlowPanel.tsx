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
}

const EMPTY_FORM: FormState = {
  mode: "closed",
  agentType: "",
  name: "",
  whenToUse: "",
  systemPrompt: "",
  model: "",
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
  const chain = useMemo(() => active?.session.agent_chain ?? [], [active]);

  const [builtins, setBuiltins] = useState<AgentSummary[]>([]);
  const [customs, setCustoms] = useState<CustomAgent[]>([]);
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

  const nameOf = useCallback(
    (agentType: string) => {
      if (agentType === MAIN) return t("agentFlow.mainName");
      if (agentType.startsWith(CUSTOM_PREFIX)) {
        const found = customs.find((c) => c.agent_type === agentType);
        return found?.name ?? agentType.slice(CUSTOM_PREFIX.length);
      }
      return agentType;
    },
    [customs, t],
  );

  const agents = useMemo(
    () => [
      ...builtins.map((b) => ({ id: b.agent_type, name: b.agent_type, custom: false })),
      ...customs.map((c) => ({ id: c.agent_type, name: c.name, custom: true })),
    ],
    [builtins, customs],
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

  const openNew = () => setForm({ ...EMPTY_FORM, mode: "new" });
  const openEditByType = useCallback(
    (agentType: string) => {
      const c = customs.find((x) => x.agent_type === agentType);
      if (!c) return;
      setForm({
        mode: "edit",
        agentType: c.agent_type,
        name: c.name,
        whenToUse: c.when_to_use,
        systemPrompt: c.system_prompt,
        model: c.model ?? "",
      });
    },
    [customs],
  );
  const closeForm = () => setForm(EMPTY_FORM);

  const submitForm = async () => {
    const name = form.name.trim();
    if (!name) return;
    try {
      if (form.mode === "new") {
        await api.createCustomAgent({
          name,
          whenToUse: form.whenToUse,
          systemPrompt: form.systemPrompt,
          model: form.model.trim() || null,
        });
      } else if (form.mode === "edit") {
        await api.updateCustomAgent({
          agentType: form.agentType,
          name,
          whenToUse: form.whenToUse,
          systemPrompt: form.systemPrompt,
          model: form.model.trim() || null,
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

  const tab = open ? 0 : -1;

  if (form.mode !== "closed") {
    return (
      <div className="agent-flow agent-flow--form">
        <div className="agent-flow-section-head">
          {form.mode === "new" ? t("agentFlow.newAgent") : t("agentFlow.editAgent")}
        </div>
        <div className="agent-flow-form">
          <label className="agent-flow-field">
            <span>{t("agentFlow.formName")}</span>
            <input
              type="text"
              value={form.name}
              tabIndex={tab}
              placeholder={t("agentFlow.formNamePlaceholder")}
              onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
            />
          </label>
          <label className="agent-flow-field">
            <span>{t("agentFlow.formWhenToUse")}</span>
            <input
              type="text"
              value={form.whenToUse}
              tabIndex={tab}
              placeholder={t("agentFlow.formWhenToUsePlaceholder")}
              onChange={(e) => setForm((f) => ({ ...f, whenToUse: e.target.value }))}
            />
          </label>
          <label className="agent-flow-field">
            <span>{t("agentFlow.formSystemPrompt")}</span>
            <textarea
              rows={6}
              value={form.systemPrompt}
              tabIndex={tab}
              placeholder={t("agentFlow.formSystemPromptPlaceholder")}
              onChange={(e) => setForm((f) => ({ ...f, systemPrompt: e.target.value }))}
            />
          </label>
          <label className="agent-flow-field">
            <span>{t("agentFlow.formModel")}</span>
            <input
              type="text"
              value={form.model}
              tabIndex={tab}
              placeholder={t("agentFlow.formModelPlaceholder")}
              onChange={(e) => setForm((f) => ({ ...f, model: e.target.value }))}
            />
          </label>
          <div className="agent-flow-form-actions">
            <button type="button" className="ghost-btn" tabIndex={tab} onClick={closeForm}>
              {t("agentFlow.cancel")}
            </button>
            <button
              type="button"
              className="btn primary"
              tabIndex={tab}
              disabled={!form.name.trim()}
              onClick={() => void submitForm()}
            >
              {form.mode === "new" ? t("agentFlow.create") : t("agentFlow.save")}
            </button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <AgentFlowCanvas
      open={open}
      sessionId={sessionId}
      chain={chain}
      agents={agents}
      knownTypes={knownTypes}
      nameOf={nameOf}
      onOrderChange={onOrderChange}
      onRequestNewAgent={openNew}
      onEditAgent={openEditByType}
      onDeleteAgent={deleteByType}
    />
  );
}
