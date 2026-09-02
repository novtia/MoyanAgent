import { useEffect, useMemo, useState, type MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import { openContextMenu } from "../../../context-menu";
import { useSettings } from "../../../../store/settings";
import type {
  LlmModelCatalog,
  ModelProvider,
  ModelServiceModel,
  RemoteModelInfo,
} from "../../../../types";
import { api } from "../../../../api/tauri";
import { toast, dialog } from "../../../ui";
import {
  getProviderSdkConfig,
  isBuiltinProvider,
  makeModel,
  makeProvider,
  normalizeProviderSdk,
  normalizeProviders,
  providerAvatar,
  remoteInfoToModelPatch,
  validateProviderConfig,
} from "../modelServices";
import { EMPTY_PROVIDER_DRAFT, type ProviderDraft } from "./types";

export function useModelService() {
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
  const [manageModelsOpen, setManageModelsOpen] = useState(false);
  const [editingModel, setEditingModel] = useState<ModelServiceModel | null>(null);
  const [addProviderOpen, setAddProviderOpen] = useState(false);
  const [addModelOpen, setAddModelOpen] = useState(false);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(() => new Set());
  const [llmCatalog, setLlmCatalog] = useState<LlmModelCatalog | null>(null);
  const [catalogError, setCatalogError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api
      .getLlmModelCatalog()
      .then((c) => {
        if (!cancelled) setLlmCatalog(c);
      })
      .catch((err) => {
        if (!cancelled) {
          setCatalogError(err instanceof Error ? err.message : String(err));
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const sdkOptions = llmCatalog?.providerSdkOptions ?? [];
  const builtinPresets = llmCatalog?.builtinProviderPresets ?? [];

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
    sdkOptions,
  );
  const selectedProviderValidation = selectedProvider
    ? validateProviderConfig(
        providerDraft,
        selectedProvider.enabled !== false,
        sdkOptions,
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
    setManageModelsOpen(false);
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

    const timer = window.setTimeout(() => {
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
    return () => window.clearTimeout(timer);
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
    const oldConfig = getProviderSdkConfig(latest.sdk, sdkOptions);
    const newConfig = getProviderSdkConfig(newSdk, sdkOptions);
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
    if (isBuiltinProvider(provider, builtinPresets)) {
      toast.warning("系统默认供应商不能删除。");
      return;
    }
    const ok = await dialog.confirm(
      `删除供应商 ${provider.name}？其 API 配置和模型列表也会删除。`,
      { type: "danger", confirmLabel: "删除", title: "删除供应商" },
    );
    if (!ok) return;

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
    const builtin = isBuiltinProvider(provider, builtinPresets);
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
    const provider = makeProvider(
      {
        name: draft.name,
        sdk: draft.sdk,
        avatar: draft.avatar,
      },
      sdkOptions,
    );
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

  const removeModelById = async (modelId: string) => {
    if (!selectedProvider) return;
    const nextModels = selectedProvider.models.filter((model) => model.id !== modelId);
    await patchProvider(
      selectedProvider.id,
      { models: nextModels },
      {
        active_provider_id: selectedProvider.id,
        model: settings?.model === modelId ? nextModels[0]?.id ?? "" : settings?.model,
      },
    );
  };

  const deleteModel = async (modelId: string) => {
    if (!selectedProvider) return;
    const ok = await dialog.confirm(`删除模型 ${modelId}？`, { type: "danger", confirmLabel: "删除" });
    if (!ok) return;
    await removeModelById(modelId);
    setEditingModel(null);
  };

  const addModelFromRemote = async (info: RemoteModelInfo | string) => {
    if (!selectedProvider) return;
    const id = typeof info === "string" ? info.trim() : info.id.trim();
    if (!id || selectedProvider.models.some((m) => m.id === id)) return;
    const patch =
      typeof info === "string" ? {} : remoteInfoToModelPatch(info);
    await patchProvider(selectedProvider.id, {
      models: [...selectedProvider.models, makeModel(id, patch)],
    });
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

  const patchDraft = (patch: Partial<ProviderDraft>) => {
    setProviderDraft((draft) => ({ ...draft, ...patch }));
  };

  return {
    catalogError,
    llmCatalog,
    sdkOptions,
    providers,
    filteredProviders,
    selectedProvider,
    providerDraft,
    providerSearch,
    showKey,
    selectedSdkConfig,
    selectedProviderValidation,
    activeProviderId,
    settingsModel: settings?.model,
    groupedModels,
    collapsedGroups,
    addProviderOpen,
    editProviderTarget,
    addModelOpen,
    manageModelsOpen,
    editingModel,
    setProviderSearch,
    patchDraft,
    toggleShowKey: () => setShowKey((value) => !value),
    toggleGroupCollapsed,
    selectProvider,
    openProviderMenu,
    setProviderEnabled,
    addProvider,
    submitNewProvider,
    saveEditedProvider,
    patchProvider,
    submitNewModel,
    saveModel,
    deleteModel,
    removeModelById,
    addModelFromRemote,
    setActiveModel,
    setAddProviderOpen,
    setEditProviderTarget,
    setAddModelOpen,
    setManageModelsOpen,
    setEditingModel,
  };
}
