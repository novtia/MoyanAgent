import { create } from "zustand";
import type { ModelUsageRow } from "../types";

export const USAGE_PRICING_STORAGE_KEY = "atelier.usage.modelPricing";

export interface ModelPrice {
  /** Uncached / fresh input tokens, CNY per 1M. */
  inputPer1M: number;
  outputPer1M: number;
  /** Cache-hit (read) tokens. `0` = bill at {@link inputPer1M}. */
  cacheReadPer1M: number;
  /** Cache-write / creation tokens. `0` = not charged in the estimate. */
  cacheWritePer1M: number;
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

export const EMPTY_MODEL_PRICE: ModelPrice = {
  inputPer1M: 0,
  outputPer1M: 0,
  cacheReadPer1M: 0,
  cacheWritePer1M: 0,
};

/** Defaults match design/usage-page.html (CNY per 1M tokens). */
export const DEFAULT_MODEL_PRICES: Record<string, ModelPrice> = {
  "claude-sonnet-4": {
    inputPer1M: 21.6,
    outputPer1M: 108,
    cacheReadPer1M: 2.16,
    cacheWritePer1M: 27,
  },
  "gpt-5": {
    inputPer1M: 9,
    outputPer1M: 72,
    cacheReadPer1M: 2.25,
    cacheWritePer1M: 0,
  },
  "gemini-2.5-pro": {
    inputPer1M: 8.9,
    outputPer1M: 71.2,
    cacheReadPer1M: 2.225,
    cacheWritePer1M: 0,
  },
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
  const cacheRead = Number(o.cacheReadPer1M);
  const cacheWrite = Number(o.cacheWritePer1M);
  return {
    inputPer1M: Math.max(0, input),
    outputPer1M: Math.max(0, output),
    cacheReadPer1M: Number.isFinite(cacheRead) ? Math.max(0, cacheRead) : 0,
    cacheWritePer1M: Number.isFinite(cacheWrite) ? Math.max(0, cacheWrite) : 0,
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

/** Claude/Anthropic report cache tokens outside `input_tokens`; others fold them in. */
function isSeparateCacheProvider(provider?: string | null): boolean {
  const p = (provider ?? "").toLowerCase();
  return p === "claude" || p.includes("anthropic");
}

/**
 * Whether cache_read is already included in `prompt_tokens` (OpenAI-style).
 * Claude reports cache outside input; unknown providers use the hit-rate heuristic.
 */
function cacheFoldedIntoPrompt(
  promptTokens: number,
  cacheReadTokens: number,
  provider?: string | null,
): boolean {
  if (cacheReadTokens <= 0) return true;
  if (isSeparateCacheProvider(provider)) return false;
  if (provider) return true;
  return cacheReadTokens <= promptTokens;
}

export interface CostEstimate {
  total: number;
  input: number;
  output: number;
  cache: number;
}

export function estimateCost(
  byModel: ModelUsageRow[],
  prices: Record<string, ModelPrice>,
): CostEstimate {
  let input = 0;
  let output = 0;
  let cache = 0;
  for (const row of byModel) {
    const price = prices[row.model];
    if (!price) continue;

    const prompt = row.prompt_tokens ?? 0;
    const completion = row.completion_tokens ?? 0;
    const cacheRead = row.cache_read_tokens ?? 0;
    const cacheWrite = row.cache_write_tokens ?? 0;
    const folded = cacheFoldedIntoPrompt(prompt, cacheRead, row.provider);

    const cacheReadRate =
      price.cacheReadPer1M > 0 ? price.cacheReadPer1M : price.inputPer1M;
    const cacheWriteRate = price.cacheWritePer1M;

    if (folded) {
      const cachedInPrompt = Math.min(cacheRead, Math.max(0, prompt));
      const fresh = Math.max(0, prompt - cachedInPrompt);
      input += (fresh / 1_000_000) * price.inputPer1M;
      cache += (cachedInPrompt / 1_000_000) * cacheReadRate;
      // Extra cache beyond prompt (rare) still billed as cache read.
      const cacheExtra = Math.max(0, cacheRead - cachedInPrompt);
      cache += (cacheExtra / 1_000_000) * cacheReadRate;
    } else {
      input += (prompt / 1_000_000) * price.inputPer1M;
      cache += (cacheRead / 1_000_000) * cacheReadRate;
    }
    cache += (cacheWrite / 1_000_000) * cacheWriteRate;
    output += (completion / 1_000_000) * price.outputPer1M;
  }
  return { total: input + output + cache, input, output, cache };
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
    const current = get().prices[model] ?? { ...EMPTY_MODEL_PRICE };
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
      prices[model] = { ...EMPTY_MODEL_PRICE };
      changed = true;
    }
    if (!changed) return;
    const next = { ...get(), prices };
    set({ prices });
    persist(next);
  },
}));
