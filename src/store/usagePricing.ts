import { create } from "zustand";
import type { ModelUsageRow } from "../types";

export const USAGE_PRICING_STORAGE_KEY = "atelier.usage.modelPricing";

export interface ModelPrice {
  inputPer1M: number;
  outputPer1M: number;
}

export interface UsagePricingState {
  enabled: boolean;
  currency: "¥";
  prices: Record<string, ModelPrice>;
}

interface UsagePricingStore extends UsagePricingState {
  setEnabled: (enabled: boolean) => void;
  setPrice: (model: string, patch: Partial<ModelPrice>) => void;
  ensureModels: (models: string[]) => void;
}

/** Defaults match design/usage-page.html (CNY per 1M tokens). */
export const DEFAULT_MODEL_PRICES: Record<string, ModelPrice> = {
  "claude-sonnet-4": { inputPer1M: 21.6, outputPer1M: 108 },
  "gpt-5": { inputPer1M: 9, outputPer1M: 72 },
  "gemini-2.5-pro": { inputPer1M: 8.9, outputPer1M: 71.2 },
};

const DEFAULT_STATE: UsagePricingState = {
  enabled: true,
  currency: "¥",
  prices: { ...DEFAULT_MODEL_PRICES },
};

function normalizePrice(v: unknown): ModelPrice | null {
  if (!v || typeof v !== "object") return null;
  const o = v as Partial<ModelPrice>;
  const input = Number(o.inputPer1M);
  const output = Number(o.outputPer1M);
  if (!Number.isFinite(input) || !Number.isFinite(output)) return null;
  return {
    inputPer1M: Math.max(0, input),
    outputPer1M: Math.max(0, output),
  };
}

function readStored(): UsagePricingState {
  try {
    const raw = window.localStorage.getItem(USAGE_PRICING_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_STATE, prices: { ...DEFAULT_MODEL_PRICES } };
    const parsed = JSON.parse(raw) as Partial<UsagePricingState>;
    const prices: Record<string, ModelPrice> = { ...DEFAULT_MODEL_PRICES };
    if (parsed.prices && typeof parsed.prices === "object") {
      for (const [model, price] of Object.entries(parsed.prices)) {
        const n = normalizePrice(price);
        if (n) prices[model] = n;
      }
    }
    return {
      enabled: parsed.enabled !== false,
      currency: "¥",
      prices,
    };
  } catch {
    return { ...DEFAULT_STATE, prices: { ...DEFAULT_MODEL_PRICES } };
  }
}

function persist(state: UsagePricingState) {
  try {
    window.localStorage.setItem(
      USAGE_PRICING_STORAGE_KEY,
      JSON.stringify({
        enabled: state.enabled,
        currency: state.currency,
        prices: state.prices,
      }),
    );
  } catch {
    // ignore persistence failures
  }
}

export function estimateCost(
  byModel: ModelUsageRow[],
  prices: Record<string, ModelPrice>,
): { total: number; input: number; output: number } {
  let input = 0;
  let output = 0;
  for (const row of byModel) {
    const price = prices[row.model];
    if (!price) continue;
    input += (row.prompt_tokens / 1_000_000) * price.inputPer1M;
    output += (row.completion_tokens / 1_000_000) * price.outputPer1M;
  }
  return { total: input + output, input, output };
}

const initial = readStored();

export const useUsagePricing = create<UsagePricingStore>((set, get) => ({
  ...initial,
  setEnabled: (enabled) => {
    const next = { ...get(), enabled };
    set({ enabled });
    persist(next);
  },
  setPrice: (model, patch) => {
    const current = get().prices[model] ?? { inputPer1M: 0, outputPer1M: 0 };
    const nextPrice = normalizePrice({ ...current, ...patch }) ?? current;
    const prices = { ...get().prices, [model]: nextPrice };
    const next = { ...get(), prices };
    set({ prices });
    persist(next);
  },
  ensureModels: (models) => {
    const prices = { ...get().prices };
    let changed = false;
    for (const model of models) {
      if (!model || prices[model]) continue;
      prices[model] = { inputPer1M: 0, outputPer1M: 0 };
      changed = true;
    }
    if (!changed) return;
    const next = { ...get(), prices };
    set({ prices });
    persist(next);
  },
}));
