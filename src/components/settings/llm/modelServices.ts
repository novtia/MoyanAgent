import type {
  ModelParamSettings,
  ModelProvider,
  ModelServiceModel,
} from "../../../types";

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
  const first = name.trim().charAt(0);
  return (first || "P").toUpperCase();
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
    id.includes("flux") ||
    id.includes("gpt-5")
  ) {
    caps.add("vision");
  }
  if (id.includes("search") || id.includes("sonar")) caps.add("web");
  if (id.includes("reason") || id.includes("thinking") || id.includes("o1") || id.includes("o3")) {
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

export function makeProvider(patch: Partial<ModelProvider> = {}): ModelProvider {
  const name = patch.name?.trim() || "新供应商";
  return {
    id: patch.id || makeLocalId("provider"),
    name,
    sdk: patch.sdk ?? "openrouter",
    endpoint: patch.endpoint ?? "",
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
    sdk: provider.sdk || "openrouter",
    models: provider.models.map((model) =>
      makeModel(model.id, {
        ...model,
        params: { ...EMPTY_MODEL_PARAMS, ...(model.params ?? {}) },
      }),
    ),
  }));
}
