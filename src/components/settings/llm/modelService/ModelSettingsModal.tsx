import { useEffect, useState } from "react";
import type { ModelServiceModel, ProviderSdkConfig } from "../../../../types";
import { makeModel, normalizeRouteProviders, shortModelName } from "../modelServices";
import { ModelMetaFields } from "./ModelMetaFields";
import { draftToPricing, parseOptionalInt, pricingToDraft } from "./pricing";
import { RouteProvidersField } from "./RouteProvidersField";
import type { PricingDraft } from "./types";

interface ModelSettingsModalProps {
  sdkConfig: ProviderSdkConfig;
  endpoint: string;
  apiKey: string;
  model: ModelServiceModel;
  existingIds: string[];
  onClose: () => void;
  onSave: (model: ModelServiceModel) => void;
  onDelete: () => void;
}

export function ModelSettingsModal({
  sdkConfig,
  endpoint,
  apiKey,
  model,
  existingIds,
  onClose,
  onSave,
  onDelete,
}: ModelSettingsModalProps) {
  const [draft, setDraft] = useState<ModelServiceModel>(() => ({ ...model }));
  const [contextWindow, setContextWindow] = useState(() =>
    model.context_window != null ? String(model.context_window) : "",
  );
  const [maxOutput, setMaxOutput] = useState(() =>
    model.max_output_tokens != null ? String(model.max_output_tokens) : "",
  );
  const [pricing, setPricing] = useState<PricingDraft>(() =>
    pricingToDraft(model.pricing),
  );

  useEffect(() => {
    setDraft({ ...model });
    setContextWindow(
      model.context_window != null ? String(model.context_window) : "",
    );
    setMaxOutput(
      model.max_output_tokens != null ? String(model.max_output_tokens) : "",
    );
    setPricing(pricingToDraft(model.pricing));
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
            <RouteProvidersField
              endpoint={endpoint}
              apiKey={apiKey}
              modelId={trimmedId}
              selected={normalizeRouteProviders(draft.route_providers)}
              onChange={(slugs) => patchDraft({ route_providers: slugs })}
            />

            <ModelMetaFields
              contextWindow={contextWindow}
              maxOutput={maxOutput}
              pricing={pricing}
              capabilities={draft.capabilities}
              onContextWindow={setContextWindow}
              onMaxOutput={setMaxOutput}
              onPricing={(patch) => setPricing((p) => ({ ...p, ...patch }))}
              onToggleCapability={toggleCapability}
            />
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
              onSave(
                makeModel(trimmedId, {
                  ...draft,
                  id: trimmedId,
                  name: draft.name.trim() || shortModelName(trimmedId),
                  group: draft.group.trim() || "custom",
                  context_window: parseOptionalInt(contextWindow) ?? null,
                  max_output_tokens: parseOptionalInt(maxOutput) ?? null,
                  pricing: draftToPricing(pricing),
                  route_providers: normalizeRouteProviders(draft.route_providers),
                }),
              )
            }
          >
            保存
          </button>
        </div>
      </div>
    </div>
  );
}
