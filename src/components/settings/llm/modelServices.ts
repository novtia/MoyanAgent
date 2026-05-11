import type {
  ModelParamSettings,
  ModelProvider,
  ModelProviderSdk,
  ModelServiceModel,
  ProviderSdkConfig,
} from "../../../types";

export type { ProviderSdkConfig };

export const DEFAULT_PROVIDER_SDK = "openai" satisfies ModelProviderSdk;

export const PROVIDER_ICON_PATHS = {
  openrouter: "/provider-icons/openrouter.svg",
  openai: "/provider-icons/openai.svg",
  gemini: "/provider-icons/gemini.svg",
  claude: "/provider-icons/claude.svg",
  grok: "/provider-icons/grok.svg",
  doubao: "/provider-icons/doubao-color.svg",
  deepseek: "/provider-icons/deepseek.svg",
} as const;

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
};

export const CAPABILITY_OPTIONS = [
  { id: "vision", label: "视觉" },
  { id: "web", label: "联网" },
  { id: "reasoning", label: "推理" },
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

export function isProviderAvatarImage(avatar?: string | null) {
  const value = avatar?.trim() ?? "";
  const lower = value.toLowerCase();
  return (
    lower.startsWith("data:image/") ||
    lower.startsWith("/") ||
    lower.startsWith("http://") ||
    lower.startsWith("https://") ||
    /\.(apng|avif|gif|jpe?g|png|svg|webp)(\?.*)?$/.test(lower)
  );
}

export function providerAvatar(provider: Pick<ModelProvider, "avatar" | "name">) {
  const avatar = provider.avatar?.trim() ?? "";
  return isProviderAvatarImage(avatar) ? avatar : "";
}

export function shortModelName(model?: string | null) {
  if (!model) return "model";
  const slash = model.lastIndexOf("/");
  return slash >= 0 ? model.slice(slash + 1) : model;
}

export function groupFromModelId(model: string) {
  return model.includes("/") ? model.split("/")[0] : "custom";
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
  if (caps.size === 0) caps.add("text");
  return Array.from(caps);
}

export function makeModel(
  id: string,
  patch: Partial<ModelServiceModel> = {},
): ModelServiceModel {
  return {
    id,
    name: shortModelName(id),
    group: groupFromModelId(id),
    capabilities: inferCapabilities(id),
    ...patch,
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
    models: patch.models ?? [],
  };
}

export function normalizeProviders(providers: ModelProvider[]) {
  return providers.map((provider) => ({
    ...provider,
    enabled: provider.enabled !== false,
    id: provider.id || makeLocalId("provider"),
    name: provider.name || "未命名供应商",
    sdk: normalizeProviderSdk(provider.sdk),
    avatar: providerAvatar(provider),
    endpoint: provider.endpoint ?? "",
    api_key: provider.api_key ?? "",
    models: provider.models.map((model) => makeModel(model.id, model)),
  }));
}
