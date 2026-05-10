import type {
  ModelParamSettings,
  ModelProvider,
  ModelProviderSdk,
  ModelServiceModel,
} from "../../../types";

export const DEFAULT_PROVIDER_SDK = "openai" satisfies ModelProviderSdk;

export const PROVIDER_ICON_PATHS = {
  openrouter: "/provider-icons/openrouter.svg",
  openai: "/provider-icons/openai.svg",
  gemini: "/provider-icons/gemini.svg",
  claude: "/provider-icons/claude.svg",
  grok: "/provider-icons/grok.svg",
  deepseek: "/provider-icons/deepseek.svg",
} as const;

export interface ProviderSdkConfig {
  id: ModelProviderSdk;
  label: string;
  description: string;
  defaultName: string;
  defaultEndpoint: string;
  endpointPlaceholder: string;
  endpointHint: string;
  apiKeyPlaceholder: string;
  apiKeyHint: string;
  modelIdPlaceholder: string;
  modelIdHint: string;
  models: ModelServiceModel[];
}

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
    id.includes("gpt-4")
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
  const { params, ...rest } = patch;
  return {
    id,
    name: shortModelName(id),
    group: groupFromModelId(id),
    capabilities: inferCapabilities(id),
    streaming: true,
    ...rest,
    params: { ...EMPTY_MODEL_PARAMS, ...(params ?? {}) },
  };
}

function sdkModel(
  id: string,
  name: string,
  group: string,
  capabilities: string[],
) {
  return makeModel(id, { name, group, capabilities });
}

export const PROVIDER_SDK_OPTIONS: readonly ProviderSdkConfig[] = [
  {
    id: "openai",
    label: "OpenAI Chat",
    description: "OpenAI Chat Completions 兼容协议，OpenRouter 也走这个 SDK。",
    defaultName: "OpenAI Chat",
    defaultEndpoint: "https://api.openai.com/v1/chat/completions",
    endpointPlaceholder: "https://.../chat/completions",
    endpointHint: "填写完整 chat/completions 地址；OpenRouter 使用 https://openrouter.ai/api/v1/chat/completions。",
    apiKeyPlaceholder: "sk-...",
    apiKeyHint: "填写该供应商的 API Key。",
    modelIdPlaceholder: "model-name",
    modelIdHint: "填写该供应商的模型 ID；OpenRouter 使用 provider/model-name。",
    models: [
      sdkModel("gpt-4o", "GPT 4o", "openai", ["vision", "text"]),
      sdkModel("gpt-4.1", "GPT 4.1", "openai", ["vision", "text"]),
    ],
  },
  {
    id: "openai-responses",
    label: "OpenAI Responses",
    description: "OpenAI Responses API，支持文本和图片输入，适合 OpenAI 原生新接口。",
    defaultName: "OpenAI",
    defaultEndpoint: "https://api.openai.com/v1/responses",
    endpointPlaceholder: "https://api.openai.com/v1/responses",
    endpointHint: "OpenAI Responses API 的完整地址。",
    apiKeyPlaceholder: "sk-...",
    apiKeyHint: "填写 OpenAI API Key。",
    modelIdPlaceholder: "gpt-4.1",
    modelIdHint: "填写 OpenAI Responses 支持的模型 ID。",
    models: [
      sdkModel("gpt-image-1.5", "GPT Image 1.5", "openai", ["vision"]),
      sdkModel("gpt-4.1", "GPT 4.1", "openai", ["vision", "text"]),
      sdkModel("gpt-4o", "GPT 4o", "openai", ["vision", "text"]),
    ],
  },
  {
    id: "gemini",
    label: "Gemini",
    description: "Google Gemini generateContent API，支持文本、图片输入和 Gemini 图片输出。",
    defaultName: "Gemini",
    defaultEndpoint:
      "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent",
    endpointPlaceholder:
      "https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent",
    endpointHint: "保留 {model} 占位符；后端会替换为当前模型 ID。",
    apiKeyPlaceholder: "AIza...",
    apiKeyHint: "填写 Gemini API Key。",
    modelIdPlaceholder: "gemini-2.5-flash-image",
    modelIdHint: "填写 Gemini 模型 ID。",
    models: [
      sdkModel("gemini-2.5-flash-image", "Gemini 2.5 Flash Image", "gemini", [
        "vision",
        "text",
      ]),
      sdkModel("gemini-2.5-flash", "Gemini 2.5 Flash", "gemini", [
        "vision",
        "text",
      ]),
      sdkModel("gemini-3-flash-preview", "Gemini 3 Flash Preview", "gemini", [
        "vision",
        "text",
        "reasoning",
      ]),
    ],
  },
  {
    id: "claude",
    label: "Claude",
    description: "Anthropic Messages API，支持文本和图片输入。",
    defaultName: "Claude",
    defaultEndpoint: "https://api.anthropic.com/v1/messages",
    endpointPlaceholder: "https://api.anthropic.com/v1/messages",
    endpointHint: "Anthropic Messages API 的完整地址。",
    apiKeyPlaceholder: "sk-ant-...",
    apiKeyHint: "填写 Anthropic API Key。",
    modelIdPlaceholder: "claude-sonnet-4-20250514",
    modelIdHint: "填写 Anthropic 模型 ID。",
    models: [
      sdkModel("claude-sonnet-4-20250514", "Claude Sonnet 4", "claude", [
        "vision",
        "text",
        "reasoning",
      ]),
      sdkModel("claude-opus-4-1-20250805", "Claude Opus 4.1", "claude", [
        "vision",
        "text",
        "reasoning",
      ]),
    ],
  },
  {
    id: "grok",
    label: "xAI Grok Image",
    description:
      "xAI 原生图片 API（/v1/images/generations 与 /v1/images/edits），非 OpenAI Chat 兼容层。",
    defaultName: "xAI Grok",
    defaultEndpoint: "https://api.x.ai/v1/images/generations",
    endpointPlaceholder: "https://api.x.ai/v1/images/generations",
    endpointHint:
      "使用 xAI 图片生成完整地址；编辑请求会自动改用同前缀下的 …/images/edits。也可填 https://api.x.ai/v1 作为前缀。",
    apiKeyPlaceholder: "xai-...",
    apiKeyHint: "填写 xAI（Grok）API Key。",
    modelIdPlaceholder: "grok-imagine-image-quality",
    modelIdHint: "填写 Grok Imagine 图片模型 ID（见 xAI 文档）。",
    models: [
      sdkModel("grok-imagine-image-quality", "Grok Imagine (quality)", "grok", [
        "vision",
        "text",
      ]),
    ],
  },
];

export const BUILTIN_PROVIDER_PRESETS: readonly ModelProvider[] = [
  makeProvider({
    id: "openrouter",
    name: "OpenRouter",
    sdk: "openai",
    avatar: PROVIDER_ICON_PATHS.openrouter,
    endpoint: "https://openrouter.ai/api/v1/chat/completions",
    enabled: true,
    models: [
      sdkModel("openai/gpt-5.4-image-2", "GPT Image 2", "openai", [
        "vision",
        "text",
      ]),
      sdkModel("google/gemini-2.5-flash-image", "Gemini 2.5 Flash Image", "google", [
        "vision",
        "text",
      ]),
    ],
  }),
  makeProvider({
    id: "openai",
    sdk: "openai-responses",
    avatar: PROVIDER_ICON_PATHS.openai,
    enabled: false,
  }),
  makeProvider({
    id: "gemini",
    sdk: "gemini",
    avatar: PROVIDER_ICON_PATHS.gemini,
    enabled: false,
  }),
  makeProvider({
    id: "claude",
    sdk: "claude",
    avatar: PROVIDER_ICON_PATHS.claude,
    enabled: false,
  }),
  makeProvider({
    id: "grok",
    name: "xAI Grok",
    sdk: "grok",
    avatar: PROVIDER_ICON_PATHS.grok,
    endpoint: "https://api.x.ai/v1/images/generations",
    enabled: false,
    models: [
      sdkModel("grok-imagine-image-quality", "Grok Imagine (quality)", "grok", [
        "vision",
        "text",
      ]),
    ],
  }),
  makeProvider({
    id: "deepseek",
    name: "DeepSeek",
    sdk: "openai",
    avatar: PROVIDER_ICON_PATHS.deepseek,
    endpoint: "https://api.deepseek.com/chat/completions",
    enabled: false,
    models: [
      sdkModel("deepseek-v4-flash", "DeepSeek V4 Flash", "deepseek", [
        "text",
        "reasoning",
      ]),
      sdkModel("deepseek-v4-pro", "DeepSeek V4 Pro", "deepseek", [
        "text",
        "reasoning",
      ]),
    ],
  }),
];

/** Matches backend `builtin_services()` ids — system-inserted providers must not be removable in the UI. */
export function isBuiltinProvider(provider: Pick<ModelProvider, "id">): boolean {
  return BUILTIN_PROVIDER_PRESETS.some((b) => b.id === provider.id);
}

export function normalizeProviderSdk(sdk?: string | null): string {
  const normalized = sdk?.trim().toLowerCase();
  if (!normalized || normalized === "openrouter" || normalized === "deepseek") {
    return DEFAULT_PROVIDER_SDK;
  }
  return normalized;
}

export function isKnownProviderSdk(sdk?: string | null): sdk is ModelProviderSdk {
  const normalized = normalizeProviderSdk(sdk);
  return PROVIDER_SDK_OPTIONS.some((option) => option.id === normalized);
}

export function getProviderSdkConfig(sdk?: string | null): ProviderSdkConfig {
  const normalized = normalizeProviderSdk(sdk);
  return (
    PROVIDER_SDK_OPTIONS.find((option) => option.id === normalized) ??
    PROVIDER_SDK_OPTIONS[0]
  );
}

export function providerSdkLabel(sdk?: string | null) {
  const normalized = normalizeProviderSdk(sdk);
  return (
    PROVIDER_SDK_OPTIONS.find((option) => option.id === normalized)?.label ??
    normalized
  );
}

export interface ProviderValidationErrors {
  sdk?: string;
  endpoint?: string;
  api_key?: string;
}

export function validateProviderConfig(
  provider: Pick<ModelProvider, "sdk" | "endpoint" | "api_key">,
  required = true,
): ProviderValidationErrors {
  const errors: ProviderValidationErrors = {};
  const sdk = normalizeProviderSdk(provider.sdk);

  if (!isKnownProviderSdk(sdk)) {
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

export function makeProvider(patch: Partial<ModelProvider> = {}): ModelProvider {
  const sdk = normalizeProviderSdk(patch.sdk);
  const sdkConfig = getProviderSdkConfig(sdk);
  const name = patch.name?.trim() || sdkConfig.defaultName || "新供应商";
  return {
    id: patch.id || makeLocalId("provider"),
    name,
    sdk,
    avatar: providerAvatar({ name, avatar: patch.avatar }),
    endpoint: patch.endpoint ?? sdkConfig.defaultEndpoint,
    api_key: patch.api_key ?? "",
    enabled: patch.enabled !== false,
    models: patch.models ?? sdkConfig.models.map((model) => makeModel(model.id, model)),
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
    models: provider.models.map((model) =>
      makeModel(model.id, {
        ...model,
        params: { ...EMPTY_MODEL_PARAMS, ...(model.params ?? {}) },
      }),
    ),
  }));
}
