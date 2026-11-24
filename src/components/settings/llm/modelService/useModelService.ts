import {
  useEffect,
  useMemo,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
} from "react";
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
  isProviderConfigured,
  makeModel,
  makeProvider,
  normalizeProviderSdk,
  normalizeProviders,
  providerAvatar,
  remoteInfoToModelPatch,
  validateProviderConfig,
} from "../modelServices";
import { EMPTY_PROVIDER_DRAFT, type ProviderDraft } from "./types";

let providerWriteChain: Promise<void> = Promise.resolve();

function enqueueProviderWrite(task: () => Promise<void>): Promise<void> {
  const run = providerWriteChain.then(task, task);
  providerWriteChain = run.catch(() => {});
  return run;
}

function draftFromProvider(provider: ModelProvider): ProviderDraft {
  return {
    name: provider.name,
    sdk: normalizeProviderSdk(provider.sdk),
    avatar: provider.avatar ?? "",
    endpoint: provider.endpoint,
    api_key: provider.api_key,
    local_server_exe: provider.local_server_exe ?? "",
    local_model_path: provider.local_model_path ?? "",
    local_context_size: provider.local_context_size ?? 0,
  };
}

function isDraftDirty(provider: ModelProvider, draft: ProviderDraft): boolean {
  const draftSdk = normalizeProviderSdk(draft.sdk);
  return (
    draft.name !== provider.name ||
    draftSdk !== normalizeProviderSdk(provider.sdk) ||
    draft.avatar !== (provider.avatar ?? "") ||
    draft.endpoint !== provider.endpoint ||
    draft.api_key !== provider.api_key ||
    (draft.local_server_exe ?? "") !== (provider.local_server_exe ?? "") ||
    (draft.local_model_path ?? "") !== (provider.local_model_path ?? "") ||
    (draft.local_context_size ?? 0) !== (provider.local_context_size ?? 0)
  );
}

function applyDraftToProvider(
  provider: ModelProvider,
  draft: ProviderDraft,
): ModelProvider {
  const nextName = draft.name.trim() || provider.name;
  return {
    ...provider,
    name: nextName,
    sdk: normalizeProviderSdk(draft.sdk),
    avatar: providerAvatar({
      name: nextName,
      avatar: draft.avatar,
    }),
    endpoint: draft.endpoint.trim(),
    api_key: draft.api_key,
    local_server_exe: (draft.local_server_exe ?? "").trim(),
    local_model_path: (draft.local_model_path ?? "").trim(),
    local_context_size: draft.local_context_size ?? 0,
  };
}

function withAutoActivationPatch(
  normalizedNext: ModelProvider[],
  updatedId: string,
  extraPatch: { active_provider_id?: string; model?: string } = {},
): { active_provider_id?: string; model?: string } {
  if (extraPatch.active_provider_id !== undefined) return {};
  const updatedProvider = normalizedNext.find((p) => p.id === updatedId);
  const currentSettings = useSettings.getState().settings;
  const currentActive = normalizedNext.find(
    (p) => p.id === currentSettings?.active_provider_id,
  );
  if (
    isProviderConfigured(updatedProvider) &&
    !isProviderConfigured(currentActive)
  ) {
    const curModel = currentSettings?.model ?? "";
    return {
      active_provider_id: updatedId,
      model: updatedProvider!.models.some((m) => m.id === curModel)
        ? curModel
        : updatedProvider!.models[0]?.id ?? "",
    };
  }
  return {};
}

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

  const draftProviderIdRef = useRef<string>("");
  const providerDraftRef = useRef<ProviderDraft>(EMPTY_PROVIDER_DRAFT);
  const pendingDraftRef = useRef<{ id: string; draft: ProviderDraft } | null>(
    null,
  );
  const debounceTimerRef = useRef<number | null>(null);

  const flushPendingDraft = () => {
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
      debounceTimerRef.current = null;
    }
    const pending = pendingDraftRef.current;
    pendingDraftRef.current = null;
    if (!pending) return Promise.resolve();
    return enqueueProviderWrite(async () => {
      const latest = normalizeProviders(
        useSettings.getState().settings?.model_services ?? [],
      );
      const prov = latest.find((p) => p.id === pending.id);
      if (!prov || !isDraftDirty(prov, pending.draft)) return;
      const next = latest.map((provider) =>
        provider.id === pending.id
          ? applyDraftToProvider(provider, pending.draft)
          : provider,
      );
      const normalizedNext = normalizeProviders(next);
      await useSettings.getState().update({
        model_services: normalizedNext,
        ...withAutoActivationPatch(normalizedNext, pending.id),
      });
    });
  };

  useEffect(() => {
    const nextId = selectedProvider?.id ?? "";
    if (nextId !== draftProviderIdRef.current) {
      void flushPendingDraft();
      draftProviderIdRef.current = nextId;
      const nextDraft = selectedProvider
        ? draftFromProvider(selectedProvider)
        : EMPTY_PROVIDER_DRAFT;
      providerDraftRef.current = nextDraft;
      setProviderDraft(nextDraft);
      return;
    }
    if (selectedProvider && !pendingDraftRef.current) {
      const nextDraft = draftFromProvider(selectedProvider);
      providerDraftRef.current = nextDraft;
      setProviderDraft(nextDraft);
    }
  }, [
    selectedProvider?.id,
    selectedProvider?.name,
    selectedProvider?.sdk,
    selectedProvider?.avatar,
    selectedProvider?.endpoint,
    selectedProvider?.api_key,
    selectedProvider?.local_server_exe,
    selectedProvider?.local_model_path,
    selectedProvider?.local_context_size,
  ]);

  useEffect(() => {
    return () => {
      void flushPendingDraft();
    };
  }, []);

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

  const persistProviders = async (
    nextProviders: ModelProvider[],
    patch: { active_provider_id?: string; model?: string } = {},
  ) => {
    await flushPendingDraft();
    await enqueueProviderWrite(async () => {
      await useSettings.getState().update({
        model_services: normalizeProviders(nextProviders),
        ...patch,
      });
    });
  };

  const selectProvider = async (provider: ModelProvider) => {
    setSelectedProviderId(provider.id);
    if (!isProviderConfigured(provider)) return;
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
    await flushPendingDraft();
    await enqueueProviderWrite(async () => {
      const currentSettings = useSettings.getState().settings;
      const latest = normalizeProviders(currentSettings?.model_services ?? []);
      const currentActiveId = currentSettings?.active_provider_id || "";
      const next = latest.map((p) =>
        p.id === providerId ? { ...p, enabled } : p,
      );
      const extra: { active_provider_id?: string; model?: string } = {};
      if (!enabled && providerId === currentActiveId) {
        const fallback =
          next.find((p) => p.id !== providerId && isProviderConfigured(p)) ??
          next.find((p) => p.enabled !== false && p.id !== providerId) ??
          next.find((p) => p.enabled !== false);
        if (fallback) {
          extra.active_provider_id = fallback.id;
          const cur = currentSettings?.model ?? "";
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
          !!currentActiveId &&
          next.some((p) => p.id === currentActiveId && p.enabled !== false);
        if (!activeOk) {
          const p = next.find((x) => x.id === providerId);
          if (p) {
            extra.active_provider_id = providerId;
            extra.model = p.models[0]?.id ?? "";
          }
        }
      }
      await useSettings.getState().update({
        model_services: normalizeProviders(next),
        ...extra,
      });
    });
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
    const enabledFallback =
      next.find((item) => isProviderConfigured(item)) ??
      next.find((item) => item.enabled !== false) ??
      null;
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
    const currentActive = providers.find((p) => p.id === activeProviderId);
    const patch = isProviderConfigured(currentActive)
      ? {}
      : {
          active_provider_id: provider.id,
          model: provider.models[0]?.id ?? "",
        };
    await persistProviders([...providers, provider], patch);
    setSelectedProviderId(provider.id);
    setAddProviderOpen(false);
  };

  const patchProvider = async (
    providerId: string,
    patch: Partial<ModelProvider>,
    extraPatch: { active_provider_id?: string; model?: string } = {},
  ) => {
    let pending: { id: string; draft: ProviderDraft } | null = null;
    if (pendingDraftRef.current?.id === providerId) {
      if (debounceTimerRef.current !== null) {
        window.clearTimeout(debounceTimerRef.current);
        debounceTimerRef.current = null;
      }
      pending = pendingDraftRef.current;
      pendingDraftRef.current = null;
    } else if (pendingDraftRef.current) {
      await flushPendingDraft();
    }
    await enqueueProviderWrite(async () => {
      const latest = normalizeProviders(
        useSettings.getState().settings?.model_services ?? [],
      );
      const next = latest.map((provider) => {
        if (provider.id !== providerId) return provider;
        const base = pending
          ? applyDraftToProvider(provider, pending.draft)
          : provider;
        return { ...base, ...patch };
      });
      const normalizedNext = normalizeProviders(next);
      await useSettings.getState().update({
        model_services: normalizedNext,
        ...withAutoActivationPatch(normalizedNext, providerId, extraPatch),
        ...extraPatch,
      });
    });
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
    const nextDraft = { ...providerDraftRef.current, ...patch };
    providerDraftRef.current = nextDraft;
    setProviderDraft(nextDraft);
    if (!selectedProvider) return;
    pendingDraftRef.current = { id: selectedProvider.id, draft: nextDraft };
    if (debounceTimerRef.current !== null) {
      window.clearTimeout(debounceTimerRef.current);
    }
    debounceTimerRef.current = window.setTimeout(() => {
      void flushPendingDraft();
    }, 480);
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
