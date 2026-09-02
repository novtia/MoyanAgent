import { useState } from "react";
import type { ModelServiceModel, ProviderSdkConfig } from "../../../../types";
import {
  groupFromModelId,
  makeModel,
  shortModelName,
} from "../modelServices";
import { ModelMetaFields } from "./ModelMetaFields";
import { draftToPricing, EMPTY_PRICING_DRAFT, parseOptionalInt } from "./pricing";
import { RouteProvidersField } from "./RouteProvidersField";
import type { PricingDraft } from "./types";

interface AddModelModalProps {
  sdkConfig: ProviderSdkConfig;
  endpoint: string;
  apiKey: string;
  existingIds: string[];
  onClose: () => void;
  onAdd: (model: ModelServiceModel) => void | Promise<void>;
}

export function AddModelModal({
  sdkConfig,
  endpoint,
  apiKey,
  existingIds,
  onClose,
  onAdd,
}: AddModelModalProps) {
  const [draftId, setDraftId] = useState("");
  const [draftName, setDraftName] = useState("");
  const [draftGroup, setDraftGroup] = useState("");
  const [contextWindow, setContextWindow] = useState("");
  const [maxOutput, setMaxOutput] = useState("");
  const [pricing, setPricing] = useState<PricingDraft>(EMPTY_PRICING_DRAFT);
  const [capabilities, setCapabilities] = useState<string[]>([]);
  const [routeProviders, setRouteProviders] = useState<string[]>([]);

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
    setCapabilities((prev) => {
      const prevAuto = prevTrimmed
        ? makeModel(prevTrimmed).capabilities.join(",")
        : "";
      if (prev.length === 0 || prev.join(",") === prevAuto) {
        return nextTrimmed ? makeModel(nextTrimmed).capabilities : [];
      }
      return prev;
    });
    setDraftId(raw);
  };

  const effectiveCaps =
    capabilities.length > 0
      ? capabilities
      : trimmedId
        ? makeModel(trimmedId).capabilities
        : [];

  const toggleCapability = (cap: string) => {
    setCapabilities((cur) => {
      const base =
        cur.length > 0
          ? cur
          : trimmedId
            ? makeModel(trimmedId).capabilities
            : [];
      return base.includes(cap)
        ? base.filter((c) => c !== cap)
        : [...base, cap];
    });
  };

  const submit = () => {
    if (!trimmedId || duplicate) return;
    const model = makeModel(trimmedId, {
      name: draftName.trim() || shortModelName(trimmedId),
      group: draftGroup.trim() || groupFromModelId(trimmedId),
      capabilities: effectiveCaps,
      context_window: parseOptionalInt(contextWindow) ?? null,
      max_output_tokens: parseOptionalInt(maxOutput) ?? null,
      pricing: draftToPricing(pricing),
      route_providers: routeProviders,
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
            <RouteProvidersField
              endpoint={endpoint}
              apiKey={apiKey}
              modelId={trimmedId}
              selected={routeProviders}
              onChange={setRouteProviders}
            />
            <ModelMetaFields
              contextWindow={contextWindow}
              maxOutput={maxOutput}
              pricing={pricing}
              capabilities={effectiveCaps}
              onContextWindow={setContextWindow}
              onMaxOutput={setMaxOutput}
              onPricing={(patch) => setPricing((p) => ({ ...p, ...patch }))}
              onToggleCapability={toggleCapability}
            />
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
