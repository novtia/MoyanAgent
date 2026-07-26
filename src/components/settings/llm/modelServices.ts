import type {
  ModelParamSettings,
  ModelProvider,
  ModelProviderSdk,
  ModelServiceModel,
  ProviderSdkConfig,
} from "../../../types";

export type { ProviderSdkConfig };

export const DEFAULT_PROVIDER_SDK = "openai" satisfies ModelProviderSdk;

const LEGACY_PROVIDER_ICON_DIR = "/provider-icons";

/**
 * Known @lobehub/icons brand ids we can render (ProviderIcon keywords + named extras).
 * Values are lowercase ids passed to ProviderBrandIcon.
 */
const KNOWN_BRAND_IDS = new Set<string>([
  "ai21",
  "ai360",
  "alibaba",
  "amazon",
  "anthropic",
  "aws",
  "azure",
  "azureai",
  "baichuan",
  "baidu",
  "bedrock",
  "bytedance",
  "cerebras",
  "chatglm",
  "claude",
  "cloudflare",
  "cohere",
  "deepinfra",
  "deepseek",
  "doubao",
  "fal",
  "fireworksai",
  "flux",
  "gemini",
  "gemma",
  "google",
  "grok",
  "groq",
  "huggingface",
  "hunyuan",
  "hyperbolic",
  "inflection",
  "internlm",
  "kimi",
  "kling",
  "liquid",
  "luma",
  "meta",
  "microsoft",
  "midjourney",
  "minimax",
  "mistral",
  "moonshot",
  "nebius",
  "novita",
  "nvidia",
  "ollama",
  "openai",
  "openrouter",
  "palm",
  "perplexity",
  "qwen",
  "replicate",
  "runway",
  "sambanova",
  "siliconcloud",
  "snowflake",
  "spark",
  "stability",
  "stepfun",
  "tencent",
  "tencentcloud",
  "togetherai",
  "upstage",
  "vertexai",
  "vidu",
  "volcengine",
  "wenxin",
  "workersai",
  "xai",
  "yi",
  "zeroone",
  "zhipu",
]);

/** Alias → lobehub brand id. */
const BRAND_ICON_ALIASES: Record<string, string> = {
  "meta-llama": "meta",
  mistralai: "mistral",
  "x-ai": "xai",
  "01-ai": "yi",
  "google-ai": "google",
  ai21labs: "ai21",
  "ai21-labs": "ai21",
  hf: "huggingface",
  "hugging-face": "huggingface",
  "amazon-bedrock": "bedrock",
  "aws-bedrock": "bedrock",
  "azure-openai": "azure",
  azureai: "azureai",
  vertex: "vertexai",
  "vertex-ai": "vertexai",
  "google-vertex": "vertexai",
  "workers-ai": "workersai",
  "cloudflare-workers": "workersai",
  volc: "volcengine",
  volces: "volcengine",
  together: "togetherai",
  fireworks: "fireworksai",
  "fireworks-ai": "fireworksai",
  字节: "bytedance",
  豆包: "doubao",
  通义: "qwen",
  通义千问: "qwen",
  智谱: "zhipu",
  月之暗面: "moonshot",
  百川: "baichuan",
  零一万物: "yi",
  讯飞: "spark",
  星火: "spark",
  文心: "wenxin",
  混元: "hunyuan",
  openrouter: "openrouter",
  openai: "openai",
  anthropic: "anthropic",
  claude: "claude",
  gemini: "gemini",
  google: "google",
  deepseek: "deepseek",
  doubao: "doubao",
  "doubao-color": "doubao",
  grok: "grok",
  xai: "xai",
  meta: "meta",
  llama: "meta",
  mistral: "mistral",
  qwen: "qwen",
  alibaba: "alibaba",
  dashscope: "qwen",
  cohere: "cohere",
  perplexity: "perplexity",
  nvidia: "nvidia",
  microsoft: "microsoft",
  amazon: "amazon",
  aws: "aws",
  bedrock: "bedrock",
  huggingface: "huggingface",
  moonshot: "moonshot",
  kimi: "kimi",
  zhipu: "zhipu",
  glm: "zhipu",
  chatglm: "chatglm",
  yi: "yi",
  zeroone: "zeroone",
  minimax: "minimax",
  baichuan: "baichuan",
  stepfun: "stepfun",
  siliconcloud: "siliconcloud",
  ollama: "ollama",
  groq: "groq",
  cerebras: "cerebras",
  ai21: "ai21",
  deepinfra: "deepinfra",
  novita: "novita",
  sambanova: "sambanova",
  cloudflare: "cloudflare",
  hunyuan: "hunyuan",
  wenxin: "wenxin",
  spark: "spark",
  bytedance: "bytedance",
  baidu: "baidu",
  tencent: "tencent",
  tencentcloud: "tencentcloud",
  internlm: "internlm",
  ai360: "ai360",
  liquid: "liquid",
  gemma: "gemma",
  fal: "fal",
  replicate: "replicate",
  stability: "stability",
  midjourney: "midjourney",
  kling: "kling",
  vidu: "vidu",
  luma: "luma",
  runway: "runway",
  flux: "flux",
  hyperbolic: "hyperbolic",
  upstage: "upstage",
  snowflake: "snowflake",
  nebius: "nebius",
  inflection: "inflection",
  workersai: "workersai",
  togetherai: "togetherai",
  fireworksai: "fireworksai",
  xiaomi: "kimi",
  xiaomimimo: "kimi",
  mimo: "kimi",
  "xiao-mi": "kimi",
  小米: "kimi",
};

function normalizeBrandKey(raw: string): string {
  return raw
    .trim()
    .toLowerCase()
    .replace(/[\s_]+/g, "-")
    .replace(/[^a-z0-9\u4e00-\u9fff./:-]/g, "");
}

function lookupBrandId(normalized: string): string {
  if (KNOWN_BRAND_IDS.has(normalized)) return normalized;
  const alias = BRAND_ICON_ALIASES[normalized];
  if (alias && KNOWN_BRAND_IDS.has(alias)) return alias;
  return "";
}

/** Resolve a brand/provider/group key to a @lobehub/icons brand id. */
export function resolveBrandIconId(key?: string | null): string {
  if (!key?.trim()) return "";
  const normalized = normalizeBrandKey(key);
  if (!normalized || normalized === "custom") return "";

  const direct = lookupBrandId(normalized);
  if (direct) return direct;

  const parts = normalized.split(/[-/.:]/).filter(Boolean);
  for (const part of parts) {
    const hit = lookupBrandId(part);
    if (hit) return hit;
  }

  for (const [aliasKey, brandId] of Object.entries(BRAND_ICON_ALIASES)) {
    if (aliasKey.length < 2) continue;
    if (normalized.includes(aliasKey) && KNOWN_BRAND_IDS.has(brandId)) {
      return brandId;
    }
  }

  return "";
}

/** Last-resort row if catalog has not loaded (empty list). */
const MINIMAL_SDK_OPTION: ProviderSdkConfig = {
  id: "openai",
  label: "OpenAI Chat",
  description: "",
  defaultName: "OpenAI Chat",
  defaultEndpoint: "https://api.openai.com/v1/chat/completions",
  endpointPlaceholder: "https://.../chat/completions",
  endpointHint: "",
  apiKeyPlaceholder: "sk-...",
  apiKeyHint: "",
  modelIdPlaceholder: "model-name",
  modelIdHint: "",
  models: [],
};

export const EMPTY_MODEL_PARAMS: ModelParamSettings = {
  temperature: null,
  top_p: null,
  max_tokens: null,
  frequency_penalty: null,
  presence_penalty: null,
  thinking_enabled: null,
  thinking_effort: null,
};

export const CAPABILITY_OPTIONS = [
  { id: "vision", label: "视觉" },
  { id: "web", label: "联网" },
  { id: "reasoning", label: "推理" },
  { id: "image", label: "生图" },
  { id: "video", label: "生视频" },
  { id: "multimodal-ref", label: "多模态参考" },
  { id: "tools", label: "工具" },
  { id: "text", label: "文本" },
] as const;

export function makeLocalId(prefix: string) {
  return `${prefix}-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

export function shortProviderMark(name: string) {
  const compact = name.trim();
  const words =
    compact.match(/[A-Z]+(?![a-z])|[A-Z]?[a-z]+|\d+|[\u4e00-\u9fff]/g) ?? [];
  if (words.length >= 2) {
    const firstWord = words[0] ?? "";
    const lastWord = words[words.length - 1] ?? "";
    return `${firstWord.charAt(0)}${lastWord.charAt(0)}`.toUpperCase();
  }
  const chars = Array.from(compact.replace(/\s+/g, ""));
  const first = chars[0] || "P";
  const last = chars.length > 1 ? chars[chars.length - 1] : "";
  return `${first}${last}`.toUpperCase();
}

/** User-uploaded or remote avatar image (not a legacy bundled path). */
export function isCustomAvatarImage(avatar?: string | null) {
  const value = avatar?.trim() ?? "";
  return (
    value.startsWith("data:image/") ||
    value.startsWith("http://") ||
    value.startsWith("https://")
  );
}

/** Legacy `/provider-icons/*.svg` paths stored in DB / settings. */
export function isBundledBrandIcon(avatar?: string | null) {
  return (avatar?.trim() ?? "").startsWith(`${LEGACY_PROVIDER_ICON_DIR}/`);
}

/**
 * Persist only custom avatar images. Bundled brands resolve at render time
 * from name/sdk via @lobehub/icons.
 */
export function providerAvatar(
  provider: Pick<ModelProvider, "avatar" | "name"> & { sdk?: string | null },
) {
  const avatar = provider.avatar?.trim() ?? "";
  if (isCustomAvatarImage(avatar)) return avatar;
  return "";
}

export function shortModelName(model?: string | null) {
  if (!model) return "model";
  const slash = model.lastIndexOf("/");
  return slash >= 0 ? model.slice(slash + 1) : model;
}

/** Prefix group from model ID: `openai/gpt-4` → `openai`; no `/` → `custom`. */
export function groupFromModelId(model: string) {
  const trimmed = model.trim();
  if (!trimmed.includes("/")) return "custom";
  const prefix = trimmed.split("/")[0]?.trim() || "custom";
  return prefix || "custom";
}

/** Brand id for a model ID via its vendor prefix (`openai/gpt-4` → openai). */
export function resolveModelBrandIconId(modelId?: string | null): string {
  if (!modelId?.trim()) return "";
  const group = groupFromModelId(modelId);
  if (group !== "custom") {
    const fromGroup = resolveBrandIconId(group);
    if (fromGroup) return fromGroup;
  }
  return resolveBrandIconId(modelId);
}

export type ManageModelsFilter = "all" | "added" | string;

export function manageGroupLabel(group: string): string {
  if (group === "all") return "全部";
  if (group === "added") return "已添加";
  if (group === "custom") return "其他";
  return group;
}

export function manageGroupMark(group: string): string {
  if (group === "custom") return "·";
  const ch = Array.from(group.trim())[0];
  return ch ? ch.toUpperCase() : "·";
}

export function resolveManageGroupIconId(group: string): string {
  if (group === "all" || group === "added" || group === "custom") return "";
  return resolveBrandIconId(group);
}

export function inferCapabilities(model: string) {
  const id = model.toLowerCase();
  const caps = new Set<string>();
  if (
    id.includes("image") ||
    id.includes("vision") ||
    id.includes("gemini") ||
    id.includes("claude") ||
    id.includes("gpt-5") ||
    id.includes("gpt-4") ||
    id.includes("seedream") ||
    id.includes("doubao")
  ) {
    caps.add("vision");
  }
  if (id.includes("search") || id.includes("sonar")) caps.add("web");
  if (
    id.includes("reason") ||
    id.includes("thinking") ||
    id.includes("o1") ||
    id.includes("o3") ||
    id.includes("opus") ||
    id.includes("sonnet") ||
    id.includes("deepseek")
  ) {
    caps.add("reasoning");
  }
  if (
    id.includes("image") ||
    id.includes("seedream") ||
    id.includes("imagine") ||
    id.includes("dall") ||
    id.includes("flux")
  ) {
    caps.add("image");
  }
  if (id.includes("seedance") || id.includes("video-generation")) {
    caps.add("video");
    if (
      id.includes("seedance-2") ||
      id.includes("seedance_2") ||
      id.includes("seedance2")
    ) {
      caps.add("multimodal-ref");
    }
  }
  if (caps.size === 0) caps.add("text");
  return Array.from(caps);
}

function normalizePricing(
  pricing?: ModelServiceModel["pricing"] | null,
): ModelServiceModel["pricing"] | null {
  if (!pricing || typeof pricing !== "object") return null;
  const num = (v: unknown): number | null => {
    const n = Number(v);
    return Number.isFinite(n) && n >= 0 ? n : null;
  };
  const inputPer1M = num(pricing.inputPer1M);
  const outputPer1M = num(pricing.outputPer1M);
  const cacheReadPer1M = num(pricing.cacheReadPer1M);
  const cacheWritePer1M = num(pricing.cacheWritePer1M);
  if (
    inputPer1M == null &&
    outputPer1M == null &&
    cacheReadPer1M == null &&
    cacheWritePer1M == null
  ) {
    return null;
  }
  return { inputPer1M, outputPer1M, cacheReadPer1M, cacheWritePer1M };
}

function normalizeModalities(v: unknown): string[] | null {
  if (!Array.isArray(v)) return null;
  const out = v
    .filter((x): x is string => typeof x === "string")
    .map((s) => s.trim().toLowerCase())
    .filter(Boolean);
  return out.length > 0 ? out : null;
}

export function makeModel(
  id: string,
  patch: Partial<ModelServiceModel> = {},
): ModelServiceModel {
  const baseCaps =
    Array.isArray(patch.capabilities) && patch.capabilities.length > 0
      ? patch.capabilities
      : inferCapabilities(id);
  const context =
    typeof patch.context_window === "number" &&
    Number.isFinite(patch.context_window) &&
    patch.context_window > 0
      ? Math.round(patch.context_window)
      : patch.context_window === null
        ? null
        : undefined;
  const maxOut =
    typeof patch.max_output_tokens === "number" &&
    Number.isFinite(patch.max_output_tokens) &&
    patch.max_output_tokens > 0
      ? Math.round(patch.max_output_tokens)
      : patch.max_output_tokens === null
        ? null
        : undefined;
  const pricing = normalizePricing(patch.pricing);
  const input_modalities = normalizeModalities(patch.input_modalities);
  const output_modalities = normalizeModalities(patch.output_modalities);

  return {
    id,
    name: patch.name?.trim() || shortModelName(id),
    group: patch.group?.trim() || groupFromModelId(id),
    capabilities: baseCaps,
    ...(context !== undefined ? { context_window: context } : {}),
    ...(maxOut !== undefined ? { max_output_tokens: maxOut } : {}),
    ...(pricing ? { pricing } : {}),
    ...(input_modalities ? { input_modalities } : {}),
    ...(output_modalities ? { output_modalities } : {}),
  };
}

/** Map a remote catalog entry into a local model patch for `makeModel`. */
export function remoteInfoToModelPatch(
  info: {
    id: string;
    name?: string | null;
    context_window?: number | null;
    max_output_tokens?: number | null;
    pricing?: ModelServiceModel["pricing"] | null;
    capabilities?: string[] | null;
    input_modalities?: string[] | null;
    output_modalities?: string[] | null;
  },
): Partial<ModelServiceModel> {
  const inferred = inferCapabilities(info.id);
  const remoteCaps = Array.isArray(info.capabilities) ? info.capabilities : [];
  const caps = Array.from(new Set([...inferred, ...remoteCaps]));
  return {
    name: info.name?.trim() || undefined,
    capabilities: caps,
    context_window: info.context_window ?? undefined,
    max_output_tokens: info.max_output_tokens ?? undefined,
    pricing: info.pricing ?? undefined,
    input_modalities: info.input_modalities ?? undefined,
    output_modalities: info.output_modalities ?? undefined,
  };
}

export function isBuiltinProvider(
  provider: Pick<ModelProvider, "id">,
  builtinPresets: readonly Pick<ModelProvider, "id">[],
): boolean {
  return builtinPresets.some((b) => b.id === provider.id);
}

export function normalizeProviderSdk(sdk?: string | null): string {
  const normalized = sdk?.trim().toLowerCase();
  if (!normalized || normalized === "openrouter" || normalized === "deepseek") {
    return DEFAULT_PROVIDER_SDK;
  }
  return normalized;
}

export function isKnownProviderSdk(
  sdk: string | null | undefined,
  sdkOptions: readonly ProviderSdkConfig[],
): sdk is ModelProviderSdk {
  const normalized = normalizeProviderSdk(sdk);
  return sdkOptions.some((option) => option.id === normalized);
}

export function getProviderSdkConfig(
  sdk: string | null | undefined,
  sdkOptions: readonly ProviderSdkConfig[],
): ProviderSdkConfig {
  const normalized = normalizeProviderSdk(sdk);
  return (
    sdkOptions.find((option) => option.id === normalized) ??
    sdkOptions[0] ??
    MINIMAL_SDK_OPTION
  );
}

export function providerSdkLabel(
  sdk: string | null | undefined,
  sdkOptions: readonly ProviderSdkConfig[],
) {
  const normalized = normalizeProviderSdk(sdk);
  return (
    sdkOptions.find((option) => option.id === normalized)?.label ?? normalized
  );
}

export interface ProviderValidationErrors {
  sdk?: string;
  endpoint?: string;
  api_key?: string;
}

export function validateProviderConfig(
  provider: Pick<ModelProvider, "sdk" | "endpoint" | "api_key">,
  required: boolean,
  sdkOptions: readonly ProviderSdkConfig[],
): ProviderValidationErrors {
  const errors: ProviderValidationErrors = {};
  const sdk = normalizeProviderSdk(provider.sdk);

  if (!isKnownProviderSdk(sdk, sdkOptions)) {
    errors.sdk = `当前后端未注册 ${sdk} SDK。`;
  }

  const endpoint = provider.endpoint.trim();
  if (!endpoint) {
    if (required) errors.endpoint = "API 地址不能为空。";
  } else {
    try {
      const url = new URL(endpoint.replace("{model}", "model"));
      if (!["http:", "https:"].includes(url.protocol)) {
        errors.endpoint = "API 地址需要以 http:// 或 https:// 开头。";
      }
    } catch {
      errors.endpoint = "请输入完整的 API URL。";
    }
  }

  if (required && !provider.api_key.trim()) {
    errors.api_key = "API 密钥不能为空。";
  }

  return errors;
}

export function makeProvider(
  patch: Partial<ModelProvider> = {},
  sdkOptions: readonly ProviderSdkConfig[],
): ModelProvider {
  const sdk = normalizeProviderSdk(patch.sdk);
  const sdkConfig = getProviderSdkConfig(sdk, sdkOptions);
  const name = patch.name?.trim() || sdkConfig.defaultName || "新供应商";
  return {
    id: patch.id || makeLocalId("provider"),
    name,
    sdk,
    avatar: providerAvatar({ name, avatar: patch.avatar }),
    endpoint: patch.endpoint ?? sdkConfig.defaultEndpoint,
    api_key: patch.api_key ?? "",
    enabled: patch.enabled !== false,
    context_cache_enabled: patch.context_cache_enabled === true,
    models: patch.models ?? [],
  };
}

export function normalizeProviders(providers: ModelProvider[]) {
  return providers.map((provider) => ({
    ...provider,
    enabled: provider.enabled !== false,
    context_cache_enabled: provider.context_cache_enabled === true,
    id: provider.id || makeLocalId("provider"),
    name: provider.name || "未命名供应商",
    sdk: normalizeProviderSdk(provider.sdk),
    avatar: providerAvatar(provider),
    endpoint: provider.endpoint ?? "",
    api_key: provider.api_key ?? "",
    models: provider.models.map((model) => makeModel(model.id, model)),
  }));
}
