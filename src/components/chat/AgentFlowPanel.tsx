import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api/tauri";
import { SESSION_AGENT_GENERAL } from "../../config/chatMode";
import { dialog } from "../ui";
import { useSession } from "../../store/session";
import { useSettings } from "../../store/settings";
import {
  AgentFlowCanvas,
  MAIN,
  type AgentFlowCanvasHandle,
  type NodeConfigTarget,
} from "./AgentFlowCanvas";
import type {
  AgentSummary,
  ChainEntry,
  CustomAgent,
  ModelProvider,
  NodeOverrides,
} from "../../types";

const CUSTOM_PREFIX = "custom:";

interface FormState {
  mode: "closed" | "new" | "edit" | "edit-node";
  /** Chain canvas node id when mode is `edit-node`. */
  nodeId?: string;
  agentType: string;
  name: string;
  whenToUse: string;
  systemPrompt: string;
  model: string;
  tools: string[];
  loading: boolean;
}

const EMPTY_FORM: FormState = {
  mode: "closed",
  agentType: "",
  name: "",
  whenToUse: "",
  systemPrompt: "",
  model: "",
  tools: [],
  loading: false,
};

type FormSection = "basic" | "prompt" | "model" | "tools";

const FORM_SECTIONS: Array<{ id: FormSection; labelKey: string; descKey: string; icon: () => JSX.Element }> = [
  { id: "basic", labelKey: "agentFlow.formNavBasic", descKey: "agentFlow.formNavBasicDesc", icon: BasicIcon },
  { id: "prompt", labelKey: "agentFlow.formNavPrompt", descKey: "agentFlow.formNavPromptDesc", icon: PromptIcon },
  { id: "model", labelKey: "agentFlow.formNavModel", descKey: "agentFlow.formNavModelDesc", icon: ModelIcon },
  { id: "tools", labelKey: "agentFlow.formNavTools", descKey: "agentFlow.formNavToolsDesc", icon: ToolsIcon },
];

const NODE_PARAM_SECTIONS = FORM_SECTIONS.filter((s) => s.id !== "basic");

function toolDescription(t: (key: string, opts?: { defaultValue?: string }) => string, name: string) {
  return t(`agentFlow.toolDescriptions.${name}`, { defaultValue: name });
}

function ModelOverrideSelect({
  value,
  disabled,
  providers,
  t,
  onChange,
}: {
  value: string;
  disabled?: boolean;
  providers: ModelProvider[];
  t: (key: string) => string;
  onChange: (model: string) => void;
}) {
  const known = providers.some((p) => p.models.some((m) => m.id === value));
  return (
    <select
      className="agent-flow-select"
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value)}
    >
      <option value="">{t("agentFlow.formModelDefault")}</option>
      {value && !known && <option value={value}>{value}</option>}
      {providers.map((p) => (
        <optgroup key={p.id} label={p.name}>
          {p.models.map((m) => (
            <option key={`${p.id}:${m.id}`} value={m.id}>
              {m.name}
            </option>
          ))}
        </optgroup>
      ))}
    </select>
  );
}

function AgentToolsList({
  tools,
  selected,
  onToggle,
  disabled,
  t,
}: {
  tools: string[];
  selected: string[];
  onToggle: (tool: string) => void;
  disabled?: boolean;
  t: (key: string, opts?: { defaultValue?: string }) => string;
}) {
  return (
    <div className="agent-flow-tools agent-flow-tools--list">
      {tools.map((tool) => (
        <label key={tool} className="agent-flow-tool-row">
          <input
            type="checkbox"
            checked={selected.includes(tool)}
            disabled={disabled}
            onChange={() => onToggle(tool)}
          />
          <span className="agent-flow-tool-name">{tool}</span>
          <span className="agent-flow-tool-desc">{toolDescription(t, tool)}</span>
        </label>
      ))}
    </div>
  );
}

function resolveDefTools(defToolsRaw: string[], defAll: boolean, allTools: string[]): string[] {
  return defAll ? [...allTools] : defToolsRaw.filter((tn) => allTools.includes(tn));
}

function resolveSelectedTools(
  ov: NodeOverrides | undefined,
  defTools: string[],
  allTools: string[],
): string[] {
  // Node-override tool semantics:
  //   undefined → inherit the agent's default tool set
  //   ["*"]     → all tools
  //   []        → NO tools (empty allow-list)
  //   [names]   → exactly those tools
  if (ov?.tools === undefined) return defTools;
  if (ov.tools.includes("*")) return [...allTools];
  return ov.tools.filter((tn) => allTools.includes(tn));
}

function chainEntryType(e: ChainEntry): string {
  return typeof e === "string" ? e : e.agent_type;
}

function entryHasOverrides(e: ChainEntry): boolean {
  if (typeof e === "string") return false;
  const ov = e.overrides;
  return (
    !!ov &&
    (ov.system_prompt !== undefined ||
      ov.model !== undefined ||
      ov.tools !== undefined)
  );
}

function resolveDefinitionAgentType(agentType: string, sessionAgentType: string): string {
  return agentType === MAIN ? sessionAgentType : agentType;
}

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
  const settings = useSettings((s) => s.settings);

  const modelProviders = useMemo(
    () =>
      (settings?.model_services ?? []).filter(
        (p) => p.enabled !== false && p.models.length > 0,
      ),
    [settings?.model_services],
  );

  const sessionId = active?.session.id ?? null;
  const projectId = active?.session.project_id ?? null;
  const flowScopeId = projectId ?? sessionId;
  const chain = useMemo(() => active?.session.agent_chain ?? [], [active]);
  const sessionAgentType = active?.session.agent_type ?? SESSION_AGENT_GENERAL;

  const [builtins, setBuiltins] = useState<AgentSummary[]>([]);
  const [customs, setCustoms] = useState<CustomAgent[]>([]);
  const [allTools, setAllTools] = useState<string[]>([]);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [formSection, setFormSection] = useState<FormSection>("basic");
  const canvasRef = useRef<AgentFlowCanvasHandle>(null);

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
    (order: ChainEntry[]) => {
      if (!sessionId) return;
      const onlyMain = order.length === 1 && chainEntryType(order[0]) === MAIN;
      void setAgentChain(onlyMain && !entryHasOverrides(order[0]) ? [] : order);
    },
    [sessionId, setAgentChain],
  );

  const closeForm = () => setForm(EMPTY_FORM);

  /** Open the full parameter editor for any chain node (including MainAgent). */
  const openNodeEditor = useCallback(
    async (target: NodeConfigTarget) => {
      setFormSection("prompt");
      setForm({
        mode: "edit-node",
        nodeId: target.nodeId,
        agentType: target.agentType,
        name: nameOf(target.agentType),
        whenToUse: "",
        systemPrompt: "",
        model: "",
        tools: [],
        loading: true,
      });
      try {
        const defAgentType = resolveDefinitionAgentType(target.agentType, sessionAgentType);
        const def = await api.getAgentDefinition(defAgentType);
        const defAll = def.tools.includes("*");
        const defTools = resolveDefTools(def.tools, defAll, allTools);
        const ov =
          canvasRef.current?.getNodeOverrides(target.nodeId) ?? target.overrides;
        setForm((f) =>
          f.mode === "edit-node" && f.nodeId === target.nodeId
            ? {
                ...f,
                systemPrompt: ov?.system_prompt ?? def.system_prompt,
                model: (ov?.model !== undefined ? ov.model ?? "" : def.model ?? "") || "",
                tools: resolveSelectedTools(ov, defTools, allTools),
                loading: false,
              }
            : f,
        );
      } catch (e) {
        console.warn(e);
        setForm((f) =>
          f.mode === "edit-node" && f.nodeId === target.nodeId ? { ...f, loading: false } : f,
        );
      }
    },
    [allTools, nameOf, sessionAgentType],
  );

  const resetNodeEditor = () => {
    if (form.mode !== "edit-node" || !form.nodeId) return;
    canvasRef.current?.applyNodeOverrides(form.nodeId, null);
    closeForm();
  };

  const openNew = () => {
    setFormSection("basic");
    setForm({ ...EMPTY_FORM, mode: "new", tools: [...allTools] });
  };

  const openEditByType = useCallback(
    (agentType: string) => {
      const c = customs.find((x) => x.agent_type === agentType);
      if (!c) return;
      setFormSection("basic");
      const tools = c.tools.length > 0 ? c.tools : [...allTools];
      setForm({
        mode: "edit",
        agentType: c.agent_type,
        name: c.name,
        whenToUse: c.when_to_use,
        systemPrompt: c.system_prompt,
        model: c.model ?? "",
        tools,
        loading: false,
      });
    },
    [customs, allTools],
  );

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
    if (form.mode === "edit-node") {
      if (!form.nodeId || form.loading) return;
      // All selected → ["*"] (canonical wildcard). Otherwise persist the exact
      // selection verbatim, so an empty selection means "no tools" rather than
      // silently falling back to full access.
      const tools = allToolsSelected ? ["*"] : [...form.tools];
      const overrides: NodeOverrides = {
        system_prompt: form.systemPrompt,
        model: form.model.trim() || null,
        tools,
      };
      canvasRef.current?.applyNodeOverrides(form.nodeId, overrides);
      closeForm();
      return;
    }

    const name = form.name.trim();
    if (!name) return;
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

  const isNodeEditor = form.mode === "edit-node";
  const navSections = isNodeEditor ? NODE_PARAM_SECTIONS : FORM_SECTIONS;
  const formTitle = isNodeEditor
    ? t("agentFlow.nodeConfigTitle", { name: form.name })
    : form.mode === "new"
      ? t("agentFlow.newAgent")
      : t("agentFlow.editAgent");
  const saveDisabled = isNodeEditor
    ? form.loading
    : !form.name.trim();

  return (
    <>
      <AgentFlowCanvas
        ref={canvasRef}
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
        onEditNodeConfig={openNodeEditor}
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
          <div
            className="modal config-modal agent-flow-editor-modal"
            role="dialog"
            aria-modal="true"
            onMouseDown={(e) => e.stopPropagation()}
          >
            <div className="modal-head">
              <h3>{formTitle}</h3>
              <button type="button" className="close" onClick={closeForm}>
                {t("agentFlow.cancel")}
              </button>
            </div>
            <div className="modal-body config-modal-body">
              <nav className="config-modal-nav">
                {navSections.map(({ id, labelKey, descKey, icon: Icon }) => (
                  <button
                    key={id}
                    type="button"
                    className={`config-modal-nav-item ${formSection === id ? "active" : ""}`}
                    onClick={() => setFormSection(id)}
                  >
                    <span className="config-modal-nav-icon">
                      <Icon />
                    </span>
                    <span className="config-modal-nav-text">
                      <span className="config-modal-nav-label">{t(labelKey)}</span>
                      <span className="config-modal-nav-desc">{t(descKey)}</span>
                    </span>
                  </button>
                ))}
                {isNodeEditor && (
                  <div className="config-modal-nav-note">{t("agentFlow.nodeConfigHint")}</div>
                )}
              </nav>
              <div className="config-modal-content" key={formSection}>
                {formSection === "basic" && !isNodeEditor && (
                  <>
                    <div className="config-modal-section-head">
                      <h4 className="config-modal-section-title">{t("agentFlow.formNavBasic")}</h4>
                      <p className="config-modal-section-desc">{t("agentFlow.formNavBasicDesc")}</p>
                    </div>
                    <div className="agent-flow-form agent-flow-form--page">
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
                    </div>
                  </>
                )}
                {formSection === "prompt" && (
                  <>
                    <div className="config-modal-section-head">
                      <h4 className="config-modal-section-title">{t("agentFlow.formNavPrompt")}</h4>
                      <p className="config-modal-section-desc">{t("agentFlow.formNavPromptDesc")}</p>
                    </div>
                    <textarea
                      className="field-input field-input--lg config-modal-prompt"
                      value={form.systemPrompt}
                      disabled={form.loading}
                      spellCheck={false}
                      placeholder={t("agentFlow.formSystemPromptPlaceholder")}
                      onChange={(e) => setForm((f) => ({ ...f, systemPrompt: e.target.value }))}
                    />
                  </>
                )}
                {formSection === "model" && (
                  <>
                    <div className="config-modal-section-head">
                      <h4 className="config-modal-section-title">{t("agentFlow.formNavModel")}</h4>
                      <p className="config-modal-section-desc">{t("agentFlow.formNavModelDesc")}</p>
                    </div>
                    <div className="agent-flow-form agent-flow-form--page">
                      <label className="agent-flow-field">
                        <span>{t("agentFlow.formModel")}</span>
                        <ModelOverrideSelect
                          value={form.model}
                          disabled={form.loading}
                          providers={modelProviders}
                          t={t}
                          onChange={(model) => setForm((f) => ({ ...f, model }))}
                        />
                        <p className="agent-flow-tools-hint">{t("agentFlow.formModelHint")}</p>
                      </label>
                    </div>
                  </>
                )}
                {formSection === "tools" && (
                  <>
                    <div className="config-modal-section-head">
                      <h4 className="config-modal-section-title">{t("agentFlow.formNavTools")}</h4>
                      <p className="config-modal-section-desc">{t("agentFlow.formNavToolsDesc")}</p>
                    </div>
                    <div className="agent-flow-field">
                      <div className="agent-flow-tools-head">
                        <span>{t("agentFlow.formTools")}</span>
                        <button
                          type="button"
                          className="agent-flow-tools-toggle"
                          onClick={toggleAllTools}
                          disabled={allTools.length === 0 || form.loading}
                        >
                          {allToolsSelected
                            ? t("agentFlow.toolsDeselectAll")
                            : t("agentFlow.toolsSelectAll")}
                        </button>
                      </div>
                      {allTools.length === 0 ? (
                        <p className="agent-flow-tools-empty">{t("agentFlow.toolsEmpty")}</p>
                      ) : (
                        <AgentToolsList
                          tools={allTools}
                          selected={form.tools}
                          onToggle={toggleTool}
                          disabled={form.loading}
                          t={t}
                        />
                      )}
                      <p className="agent-flow-tools-hint">{t("agentFlow.formToolsHint")}</p>
                    </div>
                  </>
                )}
              </div>
            </div>
            <div className="modal-foot">
              {isNodeEditor && (
                <button type="button" className="btn" onClick={resetNodeEditor}>
                  {t("agentFlow.nodeConfigReset")}
                </button>
              )}
              {isNodeEditor && <span className="agent-flow-modal-foot-spacer" />}
              <button type="button" className="btn" onClick={closeForm}>
                {t("agentFlow.cancel")}
              </button>
              <button
                type="button"
                className="btn primary"
                disabled={saveDisabled}
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

function BasicIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
      <circle cx="12" cy="7" r="4" />
    </svg>
  );
}
function PromptIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2Z" />
      <path d="M8 9h8M8 13h5" />
    </svg>
  );
}
function ModelIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2 2 7l10 5 10-5-10-5Z" />
      <path d="M2 17l10 5 10-5M2 12l10 5 10-5" />
    </svg>
  );
}
function ToolsIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76Z" />
    </svg>
  );
}
