import { getProviderSdkConfig } from "./modelServices";
import { AddModelModal } from "./modelService/AddModelModal";
import { AddProviderModal } from "./modelService/AddProviderModal";
import { EditProviderModal } from "./modelService/EditProviderModal";
import { ManageModelsModal } from "./modelService/ManageModelsModal";
import { ModelSettingsModal } from "./modelService/ModelSettingsModal";
import { ProviderDetail } from "./modelService/ProviderDetail";
import { ProviderPane } from "./modelService/ProviderPane";
import { useModelService } from "./modelService/useModelService";

export function ModelServiceSection() {
  const s = useModelService();

  if (s.catalogError) {
    return (
      <div className="model-service-card">
        <div className="model-provider-detail-inner">
          <div className="model-empty-state">
            <span className="hint is-error">无法加载模型目录：{s.catalogError}</span>
          </div>
        </div>
      </div>
    );
  }
  if (!s.llmCatalog) {
    return (
      <div className="model-service-card">
        <div className="model-provider-detail-inner">
          <div className="model-empty-state model-empty-state--center">
            加载模型目录中…
          </div>
        </div>
      </div>
    );
  }

  const selected = s.selectedProvider;

  return (
    <div className="model-service-card">
      <div className="model-service-layout">
        <ProviderPane
          providers={s.providers}
          filteredProviders={s.filteredProviders}
          selectedProviderId={selected?.id}
          providerSearch={s.providerSearch}
          sdkOptions={s.sdkOptions}
          onSearchChange={s.setProviderSearch}
          onSelect={s.selectProvider}
          onContextMenu={s.openProviderMenu}
          onSetEnabled={s.setProviderEnabled}
          onAdd={s.addProvider}
        />
        <ProviderDetail
          selectedProvider={selected}
          providerDraft={s.providerDraft}
          selectedSdkConfig={s.selectedSdkConfig}
          selectedProviderValidation={s.selectedProviderValidation}
          showKey={s.showKey}
          sdkOptions={s.sdkOptions}
          activeProviderId={s.activeProviderId}
          settingsModel={s.settingsModel}
          groupedModels={s.groupedModels}
          collapsedGroups={s.collapsedGroups}
          onDraftChange={s.patchDraft}
          onToggleShowKey={s.toggleShowKey}
          onSetEnabled={s.setProviderEnabled}
          onPatchProvider={s.patchProvider}
          onToggleGroup={s.toggleGroupCollapsed}
          onSetActiveModel={s.setActiveModel}
          onEditModel={s.setEditingModel}
          onDeleteModel={s.deleteModel}
          onOpenManage={() => s.setManageModelsOpen(true)}
          onOpenAddModel={() => s.setAddModelOpen(true)}
        />
      </div>

      {s.addProviderOpen && (
        <AddProviderModal
          sdkOptions={s.sdkOptions}
          onClose={() => s.setAddProviderOpen(false)}
          onAdd={s.submitNewProvider}
        />
      )}

      {s.editProviderTarget && (
        <EditProviderModal
          sdkOptions={s.sdkOptions}
          provider={
            s.providers.find((p) => p.id === s.editProviderTarget?.id) ??
            s.editProviderTarget
          }
          onClose={() => s.setEditProviderTarget(null)}
          onSave={s.saveEditedProvider}
        />
      )}

      {s.addModelOpen && selected && (
        <AddModelModal
          sdkConfig={getProviderSdkConfig(selected.sdk, s.sdkOptions)}
          endpoint={s.providerDraft.endpoint || selected.endpoint}
          apiKey={s.providerDraft.api_key || selected.api_key}
          existingIds={selected.models.map((m) => m.id)}
          onClose={() => s.setAddModelOpen(false)}
          onAdd={s.submitNewModel}
        />
      )}

      {s.manageModelsOpen && selected && (
        <ManageModelsModal
          providerName={s.providerDraft.name || selected.name}
          sdk={selected.sdk}
          endpoint={s.providerDraft.endpoint || selected.endpoint}
          apiKey={s.providerDraft.api_key || selected.api_key}
          localModels={selected.models}
          onAdd={s.addModelFromRemote}
          onRemove={s.removeModelById}
          onClose={() => s.setManageModelsOpen(false)}
        />
      )}

      {s.editingModel && selected && (
        <ModelSettingsModal
          sdkConfig={getProviderSdkConfig(selected.sdk, s.sdkOptions)}
          endpoint={s.providerDraft.endpoint || selected.endpoint}
          apiKey={s.providerDraft.api_key || selected.api_key}
          model={s.editingModel}
          existingIds={selected.models
            .filter((model) => model.id !== s.editingModel?.id)
            .map((model) => model.id)}
          onClose={() => s.setEditingModel(null)}
          onSave={(model) => s.saveModel(s.editingModel!.id, model)}
          onDelete={() => s.deleteModel(s.editingModel!.id)}
        />
      )}
    </div>
  );
}
