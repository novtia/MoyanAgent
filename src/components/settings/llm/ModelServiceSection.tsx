import {
  useEffect,
  useMemo,
  useState,
  type ChangeEvent,
  type MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { openContextMenu } from "../../context-menu";
import { useSettings } from "../../../store/settings";
import type { ModelProvider, ModelServiceModel } from "../../../types";
import { CheckIcon, CopyIcon } from "../icons";
import {
  CAPABILITY_OPTIONS,
  DEFAULT_PROVIDER_SDK,
  PROVIDER_SDK_OPTIONS,
  type ProviderSdkConfig,
  type ProviderValidationErrors,
  getProviderSdkConfig,
  groupFromModelId,
  isBuiltinProvider,
  isKnownProviderSdk,
  makeModel,
  makeProvider,
  normalizeProviderSdk,
  normalizeProviders,
  providerAvatar,
  providerSdkLabel,
  shortProviderMark,
  shortModelName,
  validateProviderConfig,
} from "./modelServices";

type ProviderDraft = Pick<ModelProvider, "name" | "endpoint" | "api_key"> & {
  sdk: string;
  avatar: string;
};

const EMPTY_PROVIDER_DRAFT: ProviderDraft = {
  name: "",
  sdk: DEFAULT_PROVIDER_SDK,
  avatar: "",
  endpoint: "",
  api_key: "",
};

function ProviderAvatarDisplay({
  name,
  avatar,
  className = "model-provider-avatar",
}: {
  name: string;
  avatar?: string;
  className?: string;
}) {
  const src = providerAvatar({ name, avatar });
  return (
    <span className={`${className}${src ? " has-image" : ""}`}>
      {src ? <img src={src} alt="" /> : shortProviderMark(name)}
    </span>
  );
}

function readAvatarFile(file: File) {
  return new Promise<string>((resolve, reject) => {
    if (!file.type.startsWith("image/")) {
      reject(new Error("请选择图片文件。"));
      return;
    }
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result ?? ""));
    reader.onerror = () => reject(new Error("头像图片读取失败。"));
    reader.readAsDataURL(file);
  });
}

function ProviderAvatarPicker({
  name,
  avatar,
  onChange,
}: {
  name: string;
  avatar: string;
  onChange: (avatar: string) => void;
}) {
  const hasAvatar = !!providerAvatar({ name, avatar });
  const pickAvatar = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.currentTarget.files?.[0];
    event.currentTarget.value = "";
    if (!file) return;
    try {
      onChange(await readAvatarFile(file));
    } catch (error) {
      window.alert(error instanceof Error ? error.message : "头像图片读取失败。");
    }
  };

  return (
    <div className="provider-avatar-control">
      <ProviderAvatarDisplay
        name={name}
        avatar={avatar}
        className="provider-avatar-preview-image"
      />
      <label className="btn provider-avatar-upload">
        上传图片
        <input type="file" accept="image/*" onChange={pickAvatar} />
      </label>
      {hasAvatar && (
        <button type="button" className="btn" onClick={() => onChange("")}>
          移除
        </button>
      )}
    </div>
  );
}

export function ModelServiceSection() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const [selectedProviderId, setSelectedProviderId] = useState("");
  const [providerSearch, setProviderSearch] = useState("");
  const [providerDraft, setProviderDraft] =
    useState<ProviderDraft>(EMPTY_PROVIDER_DRAFT);
  const [showKey, setShowKey] = useState(false);
  const [editProviderTarget, setEditProviderTarget] = useState<ModelProvider | null>(
    null,
  );
  const [manageMode, setManageMode] = useState(false);
  const [editingModel, setEditingModel] = useState<ModelServiceModel | null>(null);
  const [addProviderOpen, setAddProviderOpen] = useState(false);
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
  const selectedSdkConfig = getProviderSdkConfig(
    providerDraft.sdk || selectedProvider?.sdk,
  );
  const selectedProviderValidation: ProviderValidationErrors = selectedProvider
    ? validateProviderConfig(
        providerDraft,
        selectedProvider.enabled !== false,
      )
    : {};

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
      setProviderDraft(EMPTY_PROVIDER_DRAFT);
      return;
    }
    setProviderDraft({
      name: selectedProvider.name,
      sdk: normalizeProviderSdk(selectedProvider.sdk),
      avatar: selectedProvider.avatar ?? "",
      endpoint: selectedProvider.endpoint,
      api_key: selectedProvider.api_key,
    });
  }, [
    selectedProvider?.id,
    selectedProvider?.name,
    selectedProvider?.sdk,
    selectedProvider?.avatar,
    selectedProvider?.endpoint,
    selectedProvider?.api_key,
  ]);

  useEffect(() => {
    setAddModelOpen(false);
    setAddProviderOpen(false);
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
    const draftSdk = normalizeProviderSdk(providerDraft.sdk);
    const dirty =
      providerDraft.name !== selectedProvider.name ||
      draftSdk !== normalizeProviderSdk(selectedProvider.sdk) ||
      providerDraft.avatar !== (selectedProvider.avatar ?? "") ||
      providerDraft.endpoint !== selectedProvider.endpoint ||
      providerDraft.api_key !== selectedProvider.api_key;
    if (!dirty) return;

    const id = selectedProvider.id;
    const draftSnap = {
      name: providerDraft.name,
      sdk: draftSdk,
      avatar: providerDraft.avatar,
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
          draftSnap.sdk === normalizeProviderSdk(prov.sdk) &&
          draftSnap.avatar === (prov.avatar ?? "") &&
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
                sdk: draftSnap.sdk,
                avatar: providerAvatar({
                  name: draftSnap.name.trim() || provider.name,
                  avatar: draftSnap.avatar,
                }),
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
  }, [
    providerDraft.name,
    providerDraft.sdk,
    providerDraft.avatar,
    providerDraft.endpoint,
    providerDraft.api_key,
    selectedProvider,
  ]);

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

  const editProvider = (provider: ModelProvider) => {
    void selectProvider(provider);
    setEditProviderTarget(provider);
  };

  const saveEditedProvider = async (draft: {
    name: string;
    sdk: string;
    avatar: string;
  }) => {
    if (!editProviderTarget) return;
    const id = editProviderTarget.id;
    const latest =
      providers.find((p) => p.id === id) ?? editProviderTarget;
    const newSdk = normalizeProviderSdk(draft.sdk);
    const oldConfig = getProviderSdkConfig(latest.sdk);
    const newConfig = getProviderSdkConfig(newSdk);
    const endpoint = latest.endpoint.trim();
    const nextEndpoint =
      !endpoint || endpoint === oldConfig.defaultEndpoint
        ? newConfig.defaultEndpoint
        : latest.endpoint;
    const nextName = draft.name.trim() || latest.name;
    await patchProvider(id, {
      name: nextName,
      sdk: newSdk,
      avatar: providerAvatar({ name: nextName, avatar: draft.avatar }),
      endpoint: nextEndpoint,
    });
    setEditProviderTarget(null);
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

  const deleteProvider = async (provider: ModelProvider) => {
    if (isBuiltinProvider(provider)) {
      window.alert("系统默认供应商不能删除。");
      return;
    }
    if (!window.confirm(`删除供应商 ${provider.name}？其 API 配置和模型列表也会删除。`)) {
      return;
    }

    const next = providers.filter((item) => item.id !== provider.id);
    const enabledFallback = next.find((item) => item.enabled !== false) ?? null;
    const patch: { active_provider_id?: string; model?: string } = {};

    if (provider.id === activeProviderId) {
      patch.active_provider_id = enabledFallback?.id ?? "";
      patch.model = enabledFallback?.models[0]?.id ?? "";
    }

    await persistProviders(next, patch);

    if (selectedProviderId === provider.id) {
      const unchangedActive =
        provider.id === activeProviderId
          ? null
          : next.find((item) => item.id === activeProviderId) ?? null;
      setSelectedProviderId(
        unchangedActive?.id ?? enabledFallback?.id ?? next[0]?.id ?? "",
      );
    }

    if (editProviderTarget?.id === provider.id) {
      setEditProviderTarget(null);
    }
  };

  const openProviderMenu = (event: ReactMouseEvent, provider: ModelProvider) => {
    const builtin = isBuiltinProvider(provider);
    openContextMenu(event, [
      {
        id: "provider-edit",
        label: t("common.edit"),
        onSelect: () => editProvider(provider),
      },
      { type: "separator" },
      {
        id: "provider-delete",
        label: t("common.delete"),
        danger: true,
        disabled: builtin,
        onSelect: () => deleteProvider(provider),
      },
    ]);
  };

  const addProvider = async () => {
    setAddProviderOpen(true);
  };

  const submitNewProvider = async (draft: {
    name: string;
    sdk: string;
    avatar: string;
  }) => {
    const provider = makeProvider({
      name: draft.name,
      sdk: draft.sdk,
      avatar: draft.avatar,
    });
    await persistProviders([...providers, provider], {
      active_provider_id: provider.id,
      model: provider.models[0]?.id ?? "",
    });
    setSelectedProviderId(provider.id);
    setAddProviderOpen(false);
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
                  onContextMenu={(event) => openProviderMenu(event, provider)}
                >
                  <button
                    type="button"
                    className="model-provider-item-body"
                    onClick={() => selectProvider(provider)}
                  >
                    <ProviderAvatarDisplay
                      name={provider.name}
                      avatar={provider.avatar}
                    />
                    <span className="model-provider-name">
                      <span className="model-provider-name-text">{provider.name}</span>
                      <span className="model-provider-sdk">
                        {providerSdkLabel(provider.sdk)}
                      </span>
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
                  <label className="field-label">API 密钥</label>
                  <div className="input-affix">
                    <input
                      type={showKey ? "text" : "password"}
                      value={providerDraft.api_key}
                      autoComplete="off"
                      spellCheck={false}
                      placeholder={selectedSdkConfig.apiKeyPlaceholder}
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
                  <div
                    className={`hint ${
                      selectedProviderValidation.api_key ? "is-error" : ""
                    }`}
                  >
                    {selectedProviderValidation.api_key ?? selectedSdkConfig.apiKeyHint}
                  </div>
                </div>
                <div className="row">
                  <label className="field-label">API 地址</label>
                  <input
                    type="url"
                    value={providerDraft.endpoint}
                    spellCheck={false}
                    placeholder={selectedSdkConfig.endpointPlaceholder}
                    onChange={(e) =>
                      setProviderDraft((draft) => ({
                        ...draft,
                        endpoint: e.target.value,
                      }))
                    }
                  />
                  <div
                    className={`hint ${
                      selectedProviderValidation.endpoint ? "is-error" : ""
                    }`}
                  >
                    {selectedProviderValidation.endpoint ??
                      selectedSdkConfig.endpointHint}
                  </div>
                </div>
              </div>

              <div className="model-list-head">
                <div>
                  <div className="model-list-title">
                    模型 <span>{selectedProvider.models.length}</span>
                  </div>
                  <div className="model-list-desc">
                    {selectedSdkConfig.modelIdHint}
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

      {addProviderOpen && (
        <AddProviderModal
          onClose={() => setAddProviderOpen(false)}
          onAdd={submitNewProvider}
        />
      )}

      {editProviderTarget && (
        <EditProviderModal
          provider={
            providers.find((p) => p.id === editProviderTarget.id) ??
            editProviderTarget
          }
          onClose={() => setEditProviderTarget(null)}
          onSave={saveEditedProvider}
        />
      )}

      {addModelOpen && selectedProvider && (
        <AddModelModal
          sdkConfig={getProviderSdkConfig(selectedProvider.sdk)}
          existingIds={selectedProvider.models.map((m) => m.id)}
          onClose={() => setAddModelOpen(false)}
          onAdd={submitNewModel}
        />
      )}

      {editingModel && selectedProvider && (
        <ModelSettingsModal
          sdkConfig={getProviderSdkConfig(selectedProvider.sdk)}
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

interface AddProviderModalProps {
  onClose: () => void;
  onAdd: (provider: { name: string; sdk: string; avatar: string }) => void | Promise<void>;
}

function AddProviderModal({ onClose, onAdd }: AddProviderModalProps) {
  const initialConfig = getProviderSdkConfig(DEFAULT_PROVIDER_SDK);
  const [sdk, setSdk] = useState(initialConfig.id);
  const [name, setName] = useState(initialConfig.defaultName);
  const [avatar, setAvatar] = useState("");
  const [nameTouched, setNameTouched] = useState(false);

  const sdkConfig = getProviderSdkConfig(sdk);
  const canSubmit = !!name.trim() && isKnownProviderSdk(sdk);

  const changeSdk = (nextSdk: string) => {
    const nextConfig = getProviderSdkConfig(nextSdk);
    setSdk(nextConfig.id);
    if (!nameTouched) setName(nextConfig.defaultName);
  };

  const submit = () => {
    if (!canSubmit) return;
    void onAdd({
      name: name.trim(),
      sdk,
      avatar: avatar.trim(),
    });
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div
        className="modal model-settings-modal add-provider-modal"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>添加供应商</h3>
          <button type="button" className="close" onClick={onClose}>
            关闭
          </button>
        </div>
        <div className="modal-body">
          <div className="model-settings-form">
            <div className="provider-avatar-preview">
              <ProviderAvatarDisplay
                name={name.trim() || sdkConfig.defaultName}
                avatar={avatar}
                className="provider-avatar-preview-image"
              />
              <div>
                <strong>{name.trim() || sdkConfig.defaultName}</strong>
                <em>{sdkConfig.label}</em>
              </div>
            </div>
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span> 供应商名称
              </label>
              <input
                type="text"
                value={name}
                autoFocus
                onChange={(e) => {
                  setNameTouched(true);
                  setName(e.target.value);
                }}
              />
            </div>
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span> 类型（SDK）
              </label>
              <select value={sdk} onChange={(e) => changeSdk(e.target.value)}>
                {PROVIDER_SDK_OPTIONS.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.label}
                  </option>
                ))}
              </select>
              <div className="hint">{sdkConfig.description}</div>
            </div>
            <div className="row">
              <label className="field-label">供应商头像</label>
              <ProviderAvatarPicker
                name={name.trim() || sdkConfig.defaultName}
                avatar={avatar}
                onChange={setAvatar}
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

interface EditProviderModalProps {
  provider: ModelProvider;
  onClose: () => void;
  onSave: (draft: { name: string; sdk: string; avatar: string }) => void | Promise<void>;
}

function EditProviderModal({ provider, onClose, onSave }: EditProviderModalProps) {
  const [sdk, setSdk] = useState(() => normalizeProviderSdk(provider.sdk));
  const [name, setName] = useState(() => provider.name);
  const [avatar, setAvatar] = useState(() => provider.avatar ?? "");

  useEffect(() => {
    setSdk(normalizeProviderSdk(provider.sdk));
    setName(provider.name);
    setAvatar(provider.avatar ?? "");
  }, [provider.id, provider.sdk, provider.name, provider.avatar]);

  const sdkConfig = getProviderSdkConfig(sdk);
  const canSubmit = !!name.trim() && isKnownProviderSdk(sdk);

  const changeSdk = (nextSdk: string) => {
    setSdk(getProviderSdkConfig(nextSdk).id);
  };

  const submit = () => {
    if (!canSubmit) return;
    void onSave({
      name: name.trim(),
      sdk,
      avatar: avatar.trim(),
    });
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div
        className="modal model-settings-modal add-provider-modal"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>编辑供应商</h3>
          <button type="button" className="close" onClick={onClose}>
            关闭
          </button>
        </div>
        <div className="modal-body">
          <div className="model-settings-form">
            <div className="provider-avatar-preview">
              <ProviderAvatarDisplay
                name={name.trim() || sdkConfig.defaultName}
                avatar={avatar}
                className="provider-avatar-preview-image"
              />
              <div>
                <strong>{name.trim() || sdkConfig.defaultName}</strong>
                <em>{sdkConfig.label}</em>
              </div>
            </div>
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span> 供应商名称
              </label>
              <input
                type="text"
                value={name}
                autoFocus
                onChange={(e) => setName(e.target.value)}
              />
            </div>
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span> 类型（SDK）
              </label>
              <select value={sdk} onChange={(e) => changeSdk(e.target.value)}>
                {!isKnownProviderSdk(sdk) && (
                  <option value={sdk}>{sdk}（未注册）</option>
                )}
                {PROVIDER_SDK_OPTIONS.map((option) => (
                  <option key={option.id} value={option.id}>
                    {option.label}
                  </option>
                ))}
              </select>
              <div className="hint">{sdkConfig.description}</div>
            </div>
            <div className="row">
              <label className="field-label">供应商头像</label>
              <ProviderAvatarPicker
                name={name.trim() || sdkConfig.defaultName}
                avatar={avatar}
                onChange={setAvatar}
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
            保存
          </button>
        </div>
      </div>
    </div>
  );
}

interface AddModelModalProps {
  sdkConfig: ProviderSdkConfig;
  existingIds: string[];
  onClose: () => void;
  onAdd: (model: ModelServiceModel) => void | Promise<void>;
}

function AddModelModal({
  sdkConfig,
  existingIds,
  onClose,
  onAdd,
}: AddModelModalProps) {
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
                placeholder={sdkConfig.modelIdPlaceholder}
                autoFocus
                onChange={(e) => onIdChange(e.target.value)}
              />
              {duplicate && <div className="hint is-error">该模型 ID 已存在。</div>}
              {!duplicate && <div className="hint">{sdkConfig.modelIdHint}</div>}
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
  sdkConfig: ProviderSdkConfig;
  model: ModelServiceModel;
  existingIds: string[];
  onClose: () => void;
  onSave: (model: ModelServiceModel) => void;
  onDelete: () => void;
}

function ModelSettingsModal({
  sdkConfig,
  model,
  existingIds,
  onClose,
  onSave,
  onDelete,
}: ModelSettingsModalProps) {
  const [draft, setDraft] = useState<ModelServiceModel>(() => ({ ...model }));

  useEffect(() => {
    setDraft({ ...model });
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
                placeholder={sdkConfig.modelIdPlaceholder}
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
              {!duplicate && <div className="hint">{sdkConfig.modelIdHint}</div>}
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
