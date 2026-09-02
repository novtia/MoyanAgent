import type { ModelPricing } from "../../../../types";
import type { PricingDraft } from "./types";
import { EMPTY_PRICING_DRAFT } from "./types";

export { EMPTY_PRICING_DRAFT };
export type { PricingDraft };

export function pricingToDraft(p?: ModelPricing | null): PricingDraft {
  const s = (v?: number | null) =>
    v != null && Number.isFinite(v) ? String(v) : "";
  return {
    inputPer1M: s(p?.inputPer1M),
    outputPer1M: s(p?.outputPer1M),
    cacheReadPer1M: s(p?.cacheReadPer1M),
    cacheWritePer1M: s(p?.cacheWritePer1M),
  };
}

export function draftToPricing(d: PricingDraft): ModelPricing | null {
  const n = (raw: string): number | null => {
    const t = raw.trim();
    if (!t) return null;
    const v = Number(t);
    return Number.isFinite(v) && v >= 0 ? v : null;
  };
  const pricing: ModelPricing = {
    inputPer1M: n(d.inputPer1M),
    outputPer1M: n(d.outputPer1M),
    cacheReadPer1M: n(d.cacheReadPer1M),
    cacheWritePer1M: n(d.cacheWritePer1M),
  };
  if (
    pricing.inputPer1M == null &&
    pricing.outputPer1M == null &&
    pricing.cacheReadPer1M == null &&
    pricing.cacheWritePer1M == null
  ) {
    return null;
  }
  return pricing;
}

export function parseOptionalInt(raw: string): number | null | undefined {
  const t = raw.trim();
  if (!t) return null;
  const v = Number(t);
  if (!Number.isFinite(v) || v <= 0) return null;
  return Math.round(v);
}
