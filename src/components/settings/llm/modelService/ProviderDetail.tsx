import { useTranslation } from "react-i18next";
import type {
  ModelProvider,
  ModelServiceModel,
  ProviderSdkConfig,
} from "../../../../types";
import { copyText } from "../../../../utils/clipboard";
import { CheckIcon, CopyIcon } from "../../icons";
import {
  capabilityLabel,
  groupFromModelId,
  manageGroupMark,
  normalizeProviderSdk,
  providerSdkLabel,
  resolveModelBrandIconId,
  shortModelName,
  type ProviderValidationErrors,
} from "../modelServices";
import { ProviderAvatarDisplay, ProviderBrandIcon } from "../ProviderBrandIcon";
import { ChevronIcon, GearIcon, ListIcon, MinusIcon, PlusIcon } from "./icons";
import { ProviderEnableSwitch } from "./ProviderEnableSwitch";
import type { ProviderDraft } from "./types";

export function ProviderDetail({
  selectedProvider,
  providerDraft,
  selectedSdkConfig,
  selectedProviderValidation,
  showKey,
  sdkOptions,
  activeProviderId,
  settingsModel,
  groupedModels,
  collapsedGroups,
  onDraftChange,
  onToggleShowKey,
  onSetEnabled,
  onPatchProvider,
  onToggleGroup,
  onSetActiveModel,
  onEditModel,
  onDeleteModel,
  onOpenManage,
  onOpenAddModel,
}: {
  selectedProvider: ModelProvider | null;
  providerDraft: ProviderDraft;
  selectedSdkConfig: ProviderSdkConfig;
  selectedProviderValidation: ProviderValidationErrors;
  showKey: boolean;
  sdkOptions: readonly ProviderSdkConfig[];
  activeProviderId: string;
  settingsModel?: string;
  groupedModels: Array<[string, ModelServiceModel[]]>;
  collapsedGroups: Set<string>;
  onDraftChange: (patch: Partial<ProviderDraft>) => void;
  onToggleShowKey: () => void;
  onSetEnabled: (providerId: string, enabled: boolean) => void;
  onPatchProvider: (providerId: string, patch: Partial<ModelProvider>) => void;
  onToggleGroup: (group: string) => void;
  onSetActiveModel: (model: ModelServiceModel) => void;
  onEditModel: (model: ModelServiceModel) => void;
  onDeleteModel: (modelId: string) => void;
  onOpenManage: () => void;
  onOpenAddModel: () => void;
}) {
  const { t } = useTranslation();

  if (!selectedProvider) {
    return (
      <section className="model-provider-detail">
        <div className="model-provider-detail-inner">
          <div className="model-empty-state model-empty-state--center">
            添加供应商后，在这里配置 API 和模型列表。
          </div>
        </div>
      </section>
    );
  }

  return (
    <section className="model-provider-detail">
      <div className="model-provider-detail-inner">
        <div className="model-provider-hero">
          <ProviderAvatarDisplay
            name={providerDraft.name || selectedProvider.name}
            avatar={selectedProvider.avatar}
            sdk={providerDraft.sdk || selectedProvider.sdk}
          />
          <div className="model-provider-hero-text">
            <span className="model-provider-hero-name">
              {providerDraft.name || selectedProvider.name}
            </span>
            <div className="model-provider-hero-meta">
              <span className="model-provider-hero-sdk">
                {providerSdkLabel(selectedProvider.sdk, sdkOptions)}
              </span>
            </div>
          </div>
          <div className="model-provider-title-tools">
            <ProviderEnableSwitch
              enabled={selectedProvider.enabled !== false}
              onChange={(next) => onSetEnabled(selectedProvider.id, next)}
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
              onClick={() => void copyText(selectedProvider.id)}
            >
              <CopyIcon />
            </button>
          </div>
        </div>

        <div className="model-provider-config">
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
                  onChange={(e) => onDraftChange({ api_key: e.target.value })}
                />
                <button
                  type="button"
                  className="affix-btn"
                  onClick={onToggleShowKey}
                >
                  {showKey
                    ? t("settings.llm.keyHide")
                    : t("settings.llm.keyShow")}
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
                onChange={(e) => onDraftChange({ endpoint: e.target.value })}
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
            {normalizeProviderSdk(selectedProvider.sdk) ===
              "openai-responses" && (
              <div className="row model-provider-switch-row">
                <div className="model-provider-switch-head">
                  <label className="field-label">
                    {t("settings.llm.contextCacheLabel")}
                  </label>
                  <ProviderEnableSwitch
                    enabled={selectedProvider.context_cache_enabled === true}
                    onChange={(next) =>
                      onPatchProvider(selectedProvider.id, {
                        context_cache_enabled: next,
                      })
                    }
                    title={t("settings.llm.contextCacheLabel")}
                  />
                </div>
                <div className="hint">
                  {t("settings.llm.contextCacheHint")}
                </div>
              </div>
            )}
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
              className="btn"
              onClick={onOpenManage}
            >
              <ListIcon />
              <span>管理</span>
            </button>
            <button
              type="button"
              className="btn"
              onClick={onOpenAddModel}
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
                    onClick={() => onToggleGroup(group)}
                  >
                    <ProviderBrandIcon
                      className="model-group-brand"
                      model={models[0]?.id}
                      group={group}
                      fallback={manageGroupMark(group)}
                      size={16}
                    />
                    <span>{group}</span>
                    <span className="model-group-title-count">{models.length}</span>
                    <ChevronIcon />
                  </button>
                  {!collapsed && (
                    <div className="model-row-list">
                      {models.map((model) => {
                        const active =
                          selectedProvider.enabled !== false &&
                          selectedProvider.id === activeProviderId &&
                          model.id === settingsModel;
                        const hasBrand = !!resolveModelBrandIconId(model.id);
                        return (
                          <div
                            key={model.id}
                            className={`model-service-row ${active ? "active" : ""}`}
                          >
                            <button
                              type="button"
                              className="model-service-main"
                              onClick={() => onSetActiveModel(model)}
                              disabled={selectedProvider.enabled === false}
                              title={model.id}
                            >
                              <ProviderBrandIcon
                                className="model-service-glyph"
                                model={model.id}
                                fallback={
                                  hasBrand
                                    ? undefined
                                    : manageGroupMark(groupFromModelId(model.id))
                                }
                                size={16}
                              />
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
                                onClick={() => onEditModel(model)}
                              >
                                <GearIcon />
                              </button>
                              <button
                                type="button"
                                className="settings-icon-btn model-remove-btn"
                                title="删除模型"
                                onClick={() => onDeleteModel(model.id)}
                              >
                                <MinusIcon />
                              </button>
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
      </div>
    </section>
  );
}
