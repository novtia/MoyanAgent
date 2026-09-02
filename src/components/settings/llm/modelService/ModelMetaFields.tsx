import { CAPABILITY_OPTIONS } from "../modelServices";
import type { PricingDraft } from "./types";

export function ModelMetaFields({
  contextWindow,
  maxOutput,
  pricing,
  capabilities,
  onContextWindow,
  onMaxOutput,
  onPricing,
  onToggleCapability,
}: {
  contextWindow: string;
  maxOutput: string;
  pricing: PricingDraft;
  capabilities: string[];
  onContextWindow: (v: string) => void;
  onMaxOutput: (v: string) => void;
  onPricing: (patch: Partial<PricingDraft>) => void;
  onToggleCapability: (id: string) => void;
}) {
  return (
    <>
      <div className="model-modal-section">
        <div className="model-modal-section-title">上下文与输出</div>
        <div className="model-meta-grid">
          <div className="row">
            <label className="field-label">上下文窗口</label>
            <input
              type="number"
              min={0}
              step={1}
              value={contextWindow}
              placeholder="如 128000"
              onChange={(e) => onContextWindow(e.target.value)}
            />
            <div className="hint">tokens · 上下文占用环与压缩预算的依据，留空按 128000 保守估算</div>
          </div>
          <div className="row">
            <label className="field-label">最大输出</label>
            <input
              type="number"
              min={0}
              step={1}
              value={maxOutput}
              placeholder="可选"
              onChange={(e) => onMaxOutput(e.target.value)}
            />
            <div className="hint">tokens · 用于限制采样参数里的 Max Tokens，留空按 65536 兜底</div>
          </div>
        </div>
      </div>

      <div className="model-modal-section">
        <div className="model-modal-section-title">价格（每百万 tokens）</div>
        <div className="hint model-pricing-hint">
          用于统计页成本估算；币种按供应商账单自行换算，不做汇率转换。
        </div>
        <div className="model-pricing-grid">
          {(
            [
              ["inputPer1M", "输入"],
              ["outputPer1M", "输出"],
              ["cacheReadPer1M", "缓存读"],
              ["cacheWritePer1M", "缓存写"],
            ] as const
          ).map(([key, label]) => (
            <span className="model-price-field" key={key}>
              <label>{label}</label>
              <span className="box">
                <input
                  type="number"
                  min={0}
                  step="0.01"
                  value={pricing[key]}
                  placeholder="0"
                  onChange={(e) => onPricing({ [key]: e.target.value })}
                />
                <span className="per">/百万</span>
              </span>
            </span>
          ))}
        </div>
      </div>

      <div className="model-modal-section">
        <div className="model-modal-section-title">模型类型</div>
        <div className="model-capability-row">
          {CAPABILITY_OPTIONS.map((option) => (
            <button
              key={option.id}
              type="button"
              className={`model-capability-chip ${
                capabilities.includes(option.id) ? "active" : ""
              }`}
              onClick={() => onToggleCapability(option.id)}
            >
              {option.label}
            </button>
          ))}
        </div>
      </div>
    </>
  );
}
