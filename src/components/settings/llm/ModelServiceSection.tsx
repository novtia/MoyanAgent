import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";
import type { ModelProvider, ModelServiceModel } from "../../../types";
import { CheckIcon, CopyIcon } from "../icons";
import { NUMERIC_FIELDS } from "./modelParams";
import { NumericParamField } from "./NumericParamField";
import {
  CAPABILITY_OPTIONS,
  EMPTY_MODEL_PARAMS,
  groupFromModelId,
  makeModel,
  makeProvider,
  normalizeProviders,
  shortModelName,
  shortProviderMark,
} from "./modelServices";

type ProviderDraft = Pick<ModelProvider, "name" | "endpoint" | "api_key">;

export function ModelServiceSection() {
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [providerSearch, setProviderSearch] = useState("");
  const [providerDraft, setProviderDraft] = useState<ProviderDraft>({
    name: "",
    endpoint: "",
    api_key: "",
  });
  const [showKey, setShowKey] = useState(false);
  const [manageMode, setManageMode] = useState(false);
  const [editingModel, setEditingModel] = useState<ModelServiceModel | null>(null);
  const [addModelOpen, setAddModelOpen] = useState(false);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => new Set());

  const providers = useMemo(
    () => normalizeProviders(settings?.model_services ?? []),
    [settings?.model_services],
  );

  const activeProviderId = settings?.active_provider_id || "";
  const selectedProvider =
    providers.find((p) => p.id === selectedProviderId) ??
    providers.find((p) => p.id === activeProviderId) ??
    providers[0] ??
    null;

  useEffect(() => {
    if (!providers.length) {
      setSelectedProviderId("");
      return;
    }
    const next = selectedProviderId || activeProviderId || providers[0].id;
    setSelectedProviderId(
      providers.some((provider) => provider.id === next) ? next : providers[0].id,
    );
  }, [activeProviderId, providers, selectedProviderId]);

  useEffect(() => {
    if (!selectedProvider) {
      setProviderDraft({ name: "", endpoint: "", api_key: "" });
      return;
    }
    setProviderDraft({
      name: selectedProvider.name,
      endpoint: selectedProvider.endpoint,
      api_key: selectedProvider.api_key,
    });
  }, [
    selectedProvider?.id,
    selectedProvider?.name,
    selectedProvider?.endpoint,
    selectedProvider?.api_key,
  ]);

  useEffect(() => {
    setAddModelOpen(false);
    setCollapsedGroups(new Set());
  }, [selectedProvider?.id]);

  const toggleGroupCollapsed = (group: string) => {
    setCollapsedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(group)) next.delete(group);
      else next.add(group);
      return next;
    });
  };

  const filteredProviders = providers.filter((provider) =>
    provider.name.toLowerCase().includes(providerSearch.trim().toLowerCase()),
  );

  useEffect(() => {
    if (!selectedProvider) return;
    const dirty =
      providerDraft.name !== selectedProvider.name ||
      providerDraft.endpoint !== selectedProvider.endpoint ||
      providerDraft.api_key !== selectedProvider.api_key;
    if (!dirty) return;

    const id = selectedProvider.id;
    const draftSnap = {
      name: providerDraft.name,
      endpoint: providerDraft.endpoint,
      api_key: providerDraft.api_key,
    };

    const t = window.setTimeout(() => {
      void (async () => {
        const latest = normalizeProviders(
          useSettings.getState().settings?.model_services ?? [],
        );
        const prov = latest.find((p) => p.id === id);
        if (!prov) return;
        if (
          draftSnap.name === prov.name &&
          draftSnap.endpoint === prov.endpoint &&
          draftSnap.api_key === prov.api_key
        ) {
          return;
        }
        const next = latest.map((provider) =>
          provider.id === id
            ? {
                ...provider,
                name: draftSnap.name.trim() || provider.name,
                endpoint: draftSnap.endpoint.trim(),
                api_key: draftSnap.api_key,
              }
            : provider,
        );
        await useSettings.getState().update({
          model_services: normalizeProviders(next),
          active_provider_id: id,
        });
      })();
    }, 480);
    return () => window.clearTimeout(t);
  }, [providerDraft.name, providerDraft.endpoint, providerDraft.api_key, selectedProvider]);

  const persistProviders = async (
    nextProviders: ModelProvider[],
    patch: { active_provider_id?: string; model?: string } = {},
  ) => {
    await update({
      model_services: normalizeProviders(nextProviders),
      ...patch,
    });
  };

  const selectProvider = async (provider: ModelProvider) => {
    setSelectedProviderId(provider.id);
    if (provider.enabled === false) return;
    const curModel = settings?.model ?? "";
    const nextModel = provider.models.some((m) => m.id === curModel)
      ? curModel
      : provider.models[0]?.id ?? "";
    await update({
      active_provider_id: provider.id,
      model: nextModel,
    });
  };

  const setProviderEnabled = async (providerId: string, enabled: boolean) => {
    const next = providers.map((p) =>
      p.id === providerId ? { ...p, enabled } : p,
    );
    const extra: { active_provider_id?: string; model?: string } = {};
    if (!enabled && providerId === activeProviderId) {
      const fallback =
        next.find((p) => p.enabled !== false && p.id !== providerId) ??
        next.find((p) => p.enabled !== false);
      if (fallback) {
        extra.active_provider_id = fallback.id;
        const cur = settings?.model ?? "";
        extra.model = fallback.models.some((m) => m.id === cur)
          ? cur
          : fallback.models[0]?.id ?? "";
      } else {
        extra.active_provider_id = "";
        extra.model = "";
      }
    }
    if (enabled) {
      const activeOk =
        !!activeProviderId &&
        next.some((p) => p.id === activeProviderId && p.enabled !== false);
      if (!activeOk) {
        const p = next.find((x) => x.id === providerId);
        if (p) {
          extra.active_provider_id = providerId;
          extra.model = p.models[0]?.id ?? "";
        }
      }
    }
    await persistProviders(next, extra);
  };

  const addProvider = async () => {
    const provider = makeProvider({ name: `供应商 ${providers.length + 1}` });
    await persistProviders([...providers, provider], {
      active_provider_id: provider.id,
      model: "",
    });
    setSelectedProviderId(provider.id);
  };

  const patchProvider = async (
    providerId: string,
    patch: Partial<ModelProvider>,
    extraPatch: { active_provider_id?: string; model?: string } = {},
  ) => {
    const next = providers.map((provider) =>
      provider.id === providerId ? { ...provider, ...patch } : provider,
    );
    await persistProviders(next, extraPatch);
  };

  const submitNewModel = async (model: ModelServiceModel) => {
    if (!selectedProvider) return;
    if (selectedProvider.models.some((m) => m.id === model.id)) return;
    await patchProvider(
      selectedProvider.id,
      { models: [...selectedProvider.models, model] },
      { active_provider_id: selectedProvider.id, model: model.id },
    );
    setAddModelOpen(false);
  };

  const saveModel = async (oldId: string, nextModel: ModelServiceModel) => {
    if (!selectedProvider) return;
    const nextModels = selectedProvider.models.map((model) =>
      model.id === oldId ? nextModel : model,
    );
    await patchProvider(
      selectedProvider.id,
      { models: nextModels },
      {
        active_provider_id: selectedProvider.id,
        model: settings?.model === oldId ? nextModel.id : settings?.model,
      },
    );
    setEditingModel(null);
  };

  const deleteModel = async (modelId: string) => {
    if (!selectedProvider) return;
    if (!window.confirm(`删除模型 ${modelId}？`)) return;
    const nextModels = selectedProvider.models.filter((model) => model.id !== modelId);
    await patchProvider(
      selectedProvider.id,
      { models: nextModels },
      {
        active_provider_id: selectedProvider.id,
        model: settings?.model === modelId ? nextModels[0]?.id ?? "" : settings?.model,
      },
    );
    setEditingModel(null);
  };

  const setActiveModel = async (model: ModelServiceModel) => {
    if (!selectedProvider || selectedProvider.enabled === false) return;
    await update({
      active_provider_id: selectedProvider.id,
      model: model.id,
    });
  };

  const groupedModels = useMemo(() => {
    const groups = new Map<string, ModelServiceModel[]>();
    for (const model of selectedProvider?.models ?? []) {
      const group = model.group || "custom";
      groups.set(group, [...(groups.get(group) ?? []), model]);
    }
    return Array.from(groups.entries());
  }, [selectedProvider?.models]);

  return (
    <div className="model-service-card">
      <div className="model-service-layout">
        <aside className="model-provider-pane">
          <input
            type="search"
            className="model-provider-search"
            value={providerSearch}
            placeholder="搜索模型平台..."
            onChange={(e) => setProviderSearch(e.target.value)}
          />
          <div className="model-provider-list">
            {filteredProviders.map((provider) => {
              const provOn = provider.enabled !== false;
              return (
                <div
                  key={provider.id}
                  className={`model-provider-item ${
                    provider.id === selectedProvider?.id ? "active" : ""
                  } ${!provOn ? "is-disabled" : ""}`}
                >
                  <button
                    type="button"
                    className="model-provider-item-body"
                    onClick={() => selectProvider(provider)}
                  >
                    <span className="model-provider-avatar">
                      {shortProviderMark(provider.name)}
                    </span>
                    <span className="model-provider-name">
                      <span className="model-provider-name-text">{provider.name}</span>
                    </span>
                  </button>
                  <ProviderEnableSwitch
                    enabled={provOn}
                    onChange={(next) => setProviderEnabled(provider.id, next)}
                    title={provOn ? "停用供应商" : "启用供应商"}
                  />
                </div>
              );
            })}
            {providers.length === 0 && (
              <div className="model-provider-empty">还没有供应商</div>
            )}
          </div>
          <button type="button" className="btn model-provider-add" onClick={addProvider}>
            <PlusIcon />
            <span>添加供应商</span>
          </button>
        </aside>

        <section className="model-provider-detail">
          {selectedProvider ? (
            <>
              <div className="model-provider-title-row">
                <div className="model-provider-heading">
                  <span>{providerDraft.name || selectedProvider.name}</span>
                  {selectedProvider.id === activeProviderId &&
                    selectedProvider.enabled !== false && (
                      <span className="model-provider-active-badge">当前供应商</span>
                    )}
                </div>
                <div className="model-provider-title-tools">
                  <ProviderEnableSwitch
                    enabled={selectedProvider.enabled !== false}
                    onChange={(next) => setProviderEnabled(selectedProvider.id, next)}
                    title={
                      selectedProvider.enabled !== false
                        ? "停用供应商"
                        : "启用供应商"
                    }
                  />
                  <button
                    type="button"
                    className="settings-icon-btn"
                    title="复制供应商 ID"
                    onClick={() => navigator.clipboard?.writeText(selectedProvider.id)}
                  >
                    <CopyIcon />
                  </button>
                </div>
              </div>

              <div className="model-provider-fields">
                <div className="row">
                  <label className="field-label">供应商名称</label>
                  <input
                    type="text"
                    value={providerDraft.name}
                    onChange={(e) =>
                      setProviderDraft((draft) => ({
                        ...draft,
                        name: e.target.value,
                      }))
                    }
                  />
                </div>
                <div className="row">
                  <label className="field-label">API 密钥</label>
                  <div className="input-affix">
                    <input
                      type={showKey ? "text" : "password"}
                      value={providerDraft.api_key}
                      autoComplete="off"
                      spellCheck={false}
                      placeholder="sk-..."
                      onChange={(e) =>
                        setProviderDraft((draft) => ({
                          ...draft,
                          api_key: e.target.value,
                        }))
                      }
                    />
                    <button
                      type="button"
                      className="affix-btn"
                      onClick={() => setShowKey((value) => !value)}
                    >
                      {showKey ? "隐藏" : "显示"}
                    </button>
                  </div>
                </div>
                <div className="row">
                  <label className="field-label">API 地址</label>
                  <input
                    type="url"
                    value={providerDraft.endpoint}
                    spellCheck={false}
                    placeholder="https://.../chat/completions"
                    onChange={(e) =>
                      setProviderDraft((draft) => ({
                        ...draft,
                        endpoint: e.target.value,
                      }))
                    }
                  />
                </div>
              </div>

              <div className="model-list-head">
                <div>
                  <div className="model-list-title">
                    模型 <span>{selectedProvider.models.length}</span>
                  </div>
                  <div className="model-list-desc">
                    点击模型设为当前使用，设置按钮打开该模型的独立参数。
                  </div>
                </div>
                <div className="model-list-actions">
                  <button
                    type="button"
                    className={`btn ${manageMode ? "primary" : ""}`}
                    onClick={() => setManageMode((value) => !value)}
                  >
                    <ListIcon />
                    <span>管理</span>
                  </button>
                  <button
                    type="button"
                    className="btn"
                    onClick={() => setAddModelOpen(true)}
                  >
                    <PlusIcon />
                    <span>添加模型</span>
                  </button>
                </div>
              </div>

              {groupedModels.length > 0 ? (
                <div className="model-group-list">
                  {groupedModels.map(([group, models]) => {
                    const collapsed = collapsedGroups.has(group);
                    return (
                    <div className="model-group" key={group}>
                      <button
                        type="button"
                        className={`model-group-title ${collapsed ? "is-collapsed" : ""}`}
                        aria-expanded={!collapsed}
                        onClick={() => toggleGroupCollapsed(group)}
                      >
                        <ChevronIcon />
                        <span>{group}</span>
                      </button>
                      {!collapsed && (
                      <div className="model-row-list">
                        {models.map((model) => {
                          const active =
                            selectedProvider.enabled !== false &&
                            selectedProvider.id === activeProviderId &&
                            model.id === settings?.model;
                          return (
                            <div
                              key={model.id}
                              className={`model-service-row ${active ? "active" : ""}`}
                            >
                              <button
                                type="button"
                                className="model-service-main"
                                onClick={() => setActiveModel(model)}
                                disabled={selectedProvider.enabled === false}
                                title={model.id}
                              >
                                <span className="model-service-glyph">✦</span>
                                <span className="model-service-text">
                                  <strong>{model.name || shortModelName(model.id)}</strong>
                                  <span>{model.id}</span>
                                </span>
                                <span className="model-service-badges">
                                  {model.capabilities.map((capability) => (
                                    <span key={capability}>
                                      {capabilityLabel(capability)}
                                    </span>
                                  ))}
                                </span>
                                {active && (
                                  <span className="model-service-current">
                                    <CheckIcon />
                                    当前
                                  </span>
                                )}
                              </button>
                              <div className="model-service-row-actions">
                                <button
                                  type="button"
                                  className="settings-icon-btn"
                                  title="模型设置"
                                  onClick={() => setEditingModel(model)}
                                >
                                  <GearIcon />
                                </button>
                                {manageMode && (
                                  <button
                                    type="button"
                                    className="settings-icon-btn danger"
                                    title="删除模型"
                                    onClick={() => deleteModel(model.id)}
                                  >
                                    <TrashIcon />
                                  </button>
                                )}
                              </div>
                            </div>
                          );
                        })}
                      </div>
                      )}
                    </div>
                    );
                  })}
                </div>
              ) : (
                <div className="model-empty-state">该供应商还没有模型。</div>
              )}
            </>
          ) : (
            <div className="model-empty-state model-empty-state--center">
              添加供应商后，在这里配置 API 和模型列表。
            </div>
          )}
        </section>
      </div>

      {addModelOpen && selectedProvider && (
        <AddModelModal
          existingIds={selectedProvider.models.map((m) => m.id)}
          onClose={() => setAddModelOpen(false)}
          onAdd={submitNewModel}
        />
      )}

      {editingModel && selectedProvider && (
        <ModelSettingsModal
          model={editingModel}
          existingIds={selectedProvider.models
            .filter((model) => model.id !== editingModel.id)
            .map((model) => model.id)}
          onClose={() => setEditingModel(null)}
          onSave={(model) => saveModel(editingModel.id, model)}
          onDelete={() => deleteModel(editingModel.id)}
        />
      )}
    </div>
  );
}

interface AddModelModalProps {
  existingIds: string[];
  onClose: () => void;
  onAdd: (model: ModelServiceModel) => void | Promise<void>;
}

function AddModelModal({ existingIds, onClose, onAdd }: AddModelModalProps) {
  const [draftId, setDraftId] = useState("");
  const [draftName, setDraftName] = useState("");
  const [draftGroup, setDraftGroup] = useState("");

  const trimmedId = draftId.trim();
  const duplicate = !!trimmedId && existingIds.includes(trimmedId);
  const canSubmit = !!trimmedId && !duplicate;

  const onIdChange = (raw: string) => {
    const prevTrimmed = draftId.trim();
    const nextTrimmed = raw.trim();
    setDraftName((prev) => {
      const prevAuto = prevTrimmed ? shortModelName(prevTrimmed) : "";
      if (prev === "" || prev === prevAuto) {
        return nextTrimmed ? shortModelName(nextTrimmed) : "";
      }
      return prev;
    });
    setDraftGroup((prev) => {
      const prevAuto = prevTrimmed ? groupFromModelId(prevTrimmed) : "";
      if (prev === "" || prev === prevAuto) {
        return nextTrimmed ? groupFromModelId(nextTrimmed) : "";
      }
      return prev;
    });
    setDraftId(raw);
  };

  const submit = () => {
    if (!trimmedId || duplicate) return;
    const model = makeModel(trimmedId, {
      name: draftName.trim() || shortModelName(trimmedId),
      group: draftGroup.trim() || groupFromModelId(trimmedId),
    });
    void onAdd(model);
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div
        className="modal model-settings-modal add-model-modal"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>添加模型</h3>
          <button type="button" className="close" onClick={onClose}>
            关闭
          </button>
        </div>
        <div className="modal-body">
          <div className="model-settings-form">
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span> 模型 ID
              </label>
              <input
                type="text"
                value={draftId}
                spellCheck={false}
                placeholder="provider/model-name"
                autoFocus
                onChange={(e) => onIdChange(e.target.value)}
              />
              {duplicate && <div className="hint is-error">该模型 ID 已存在。</div>}
            </div>
            <div className="row">
              <label className="field-label">模型名称</label>
              <input
                type="text"
                value={draftName}
                placeholder="留空则使用 ID 后缀"
                onChange={(e) => setDraftName(e.target.value)}
              />
            </div>
            <div className="row">
              <label className="field-label">分组名称</label>
              <input
                type="text"
                value={draftGroup}
                placeholder="留空则从 ID 推导（无斜杠时为 custom）"
                onChange={(e) => setDraftGroup(e.target.value)}
              />
            </div>
          </div>
        </div>
        <div className="modal-foot">
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="btn primary"
            disabled={!canSubmit}
            onClick={submit}
          >
            添加
          </button>
        </div>
      </div>
    </div>
  );
}

interface ModelSettingsModalProps {
  model: ModelServiceModel;
  existingIds: string[];
  onClose: () => void;
  onSave: (model: ModelServiceModel) => void;
  onDelete: () => void;
}

function ModelSettingsModal({
  model,
  existingIds,
  onClose,
  onSave,
  onDelete,
}: ModelSettingsModalProps) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<ModelServiceModel>(() => ({
    ...model,
    params: { ...EMPTY_MODEL_PARAMS, ...(model.params ?? {}) },
  }));

  useEffect(() => {
    setDraft({
      ...model,
      params: { ...EMPTY_MODEL_PARAMS, ...(model.params ?? {}) },
    });
  }, [model]);

  const trimmedId = draft.id.trim();
  const duplicate = existingIds.includes(trimmedId);
  const canSave = !!trimmedId && !duplicate;

  const patchDraft = (patch: Partial<ModelServiceModel>) => {
    setDraft((current) => ({ ...current, ...patch }));
  };

  const toggleCapability = (capability: string) => {
    setDraft((current) => {
      const enabled = current.capabilities.includes(capability);
      return {
        ...current,
        capabilities: enabled
          ? current.capabilities.filter((item) => item !== capability)
          : [...current.capabilities, capability],
      };
    });
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div className="modal model-settings-modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>编辑模型</h3>
          <button type="button" className="close" onClick={onClose}>
            关闭
          </button>
        </div>
        <div className="modal-body">
          <div className="model-settings-form">
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span> 模型 ID
              </label>
              <input
                type="text"
                value={draft.id}
                spellCheck={false}
                placeholder="provider/model-name"
                onChange={(e) =>
                  patchDraft({
                    id: e.target.value,
                    name:
                      draft.name === shortModelName(draft.id)
                        ? shortModelName(e.target.value)
                        : draft.name,
                  })
                }
              />
              {duplicate && <div className="hint is-error">该模型 ID 已存在。</div>}
            </div>
            <div className="row">
              <label className="field-label">模型名称</label>
              <input
                type="text"
                value={draft.name}
                onChange={(e) => patchDraft({ name: e.target.value })}
              />
            </div>
            <div className="row">
              <label className="field-label">分组名称</label>
              <input
                type="text"
                value={draft.group}
                onChange={(e) => patchDraft({ group: e.target.value })}
              />
            </div>

            <div className="model-modal-section">
              <div className="model-modal-section-title">模型类型</div>
              <div className="model-capability-row">
                {CAPABILITY_OPTIONS.map((option) => (
                  <button
                    key={option.id}
                    type="button"
                    className={`model-capability-chip ${
                      draft.capabilities.includes(option.id) ? "active" : ""
                    }`}
                    onClick={() => toggleCapability(option.id)}
                  >
                    {option.label}
                  </button>
                ))}
              </div>
            </div>

            <div className="model-modal-section model-modal-toggle-row">
              <span>支持增量文本输出</span>
              <button
                type="button"
                className={`model-service-switch ${draft.streaming ? "on" : ""}`}
                onClick={() => patchDraft({ streaming: !draft.streaming })}
              >
                <span />
              </button>
            </div>

            <div className="model-modal-section">
              <div className="model-modal-section-title">模型参数</div>
              <div className="settings-params-grid">
                {NUMERIC_FIELDS.map((field) => (
                  <NumericParamField
                    key={field.key}
                    def={field}
                    value={draft.params[field.key]}
                    onCommit={(next) =>
                      setDraft((current) => ({
                        ...current,
                        params: { ...current.params, [field.key]: next },
                      }))
                    }
                    invalidLabel={t("settings.llm.paramInvalid")}
                    label={t(field.labelKey)}
                    hint={t(field.hintKey)}
                  />
                ))}
              </div>
            </div>
          </div>
        </div>
        <div className="modal-foot">
          <button type="button" className="btn danger" onClick={onDelete}>
            删除模型
          </button>
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="btn primary"
            disabled={!canSave}
            onClick={() =>
              onSave({
                ...draft,
                id: trimmedId,
                name: draft.name.trim() || shortModelName(trimmedId),
                group: draft.group.trim() || "custom",
                params: { ...EMPTY_MODEL_PARAMS, ...(draft.params ?? {}) },
              })
            }
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}

function ProviderEnableSwitch({
  enabled,
  onChange,
  title,
}: {
  enabled: boolean;
  onChange: (enabled: boolean) => void;
  title?: string;
}) {
  return (
    <button
      type="button"
      className={`settings-toggle ${enabled ? "settings-toggle--on" : ""}`}
      role="switch"
      aria-checked={enabled}
      title={title}
      aria-label={title}
      onClick={(e) => {
        e.stopPropagation();
        onChange(!enabled);
      }}
    >
      <span className="settings-toggle-thumb" />
    </button>
  );
}

function capabilityLabel(capability: string) {
  return CAPABILITY_OPTIONS.find((option) => option.id === capability)?.label ?? capability;
}

function PlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
      <line x1="12" y1="5" x2="12" y2="19" />
      <line x1="5" y1="12" x2="19" y2="12" />
    </svg>
  );
}

function ListIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <path d="M8 6h12M8 12h12M8 18h12" />
      <path d="M4 6h.01M4 12h.01M4 18h.01" />
    </svg>
  );
}

function ChevronIcon() {
  return (
    <svg viewBox="0 0 12 12" width="12" height="12" aria-hidden>
      <path d="M4 2.5 7.5 6 4 9.5" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" />
    </svg>
  );
}

function GearIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h0a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h0a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v0a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    </svg>
  );
}
