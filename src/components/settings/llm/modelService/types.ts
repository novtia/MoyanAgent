import type { ModelProvider } from "../../../../types";
import { DEFAULT_PROVIDER_SDK } from "../modelServices";

export type ProviderDraft = Pick<ModelProvider, "name" | "endpoint" | "api_key"> & {
  sdk: string;
  avatar: string;
};

export const EMPTY_PROVIDER_DRAFT: ProviderDraft = {
  name: "",
  sdk: DEFAULT_PROVIDER_SDK,
  avatar: "",
  endpoint: "",
  api_key: "",
};

export type PricingDraft = {
  inputPer1M: string;
  outputPer1M: string;
  cacheReadPer1M: string;
  cacheWritePer1M: string;
};

export const EMPTY_PRICING_DRAFT: PricingDraft = {
  inputPer1M: "",
  outputPer1M: "",
  cacheReadPer1M: "",
  cacheWritePer1M: "",
};
