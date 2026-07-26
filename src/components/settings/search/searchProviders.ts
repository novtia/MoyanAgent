import type { WebSearchApiKind, WebSearchProviderConfig } from "../../../types";

export const SEARCH_API_KINDS: readonly WebSearchApiKind[] = [
  "tavily",
  "serper",
  "bing",
] as const;

export interface SearchKindMeta {
  id: WebSearchApiKind;
  label: string;
  description: string;
  defaultName: string;
  defaultEndpoint: string;
  endpointPlaceholder: string;
  apiKeyPlaceholder: string;
}

export const SEARCH_KIND_META: Record<WebSearchApiKind, SearchKindMeta> = {
  tavily: {
    id: "tavily",
    label: "Tavily",
    description: "Tavily Search API，面向 AI Agent 的网页检索。",
    defaultName: "Tavily",
    defaultEndpoint: "https://api.tavily.com/search",
    endpointPlaceholder: "https://api.tavily.com/search",
    apiKeyPlaceholder: "tvly-...",
  },
  serper: {
    id: "serper",
    label: "Serper",
    description: "Serper.dev Google Search API。",
    defaultName: "Serper",
    defaultEndpoint: "https://google.serper.dev/search",
    endpointPlaceholder: "https://google.serper.dev/search",
    apiKeyPlaceholder: "API Key",
  },
  bing: {
    id: "bing",
    label: "Bing API",
    description: "Microsoft Bing Web Search API v7。",
    defaultName: "Bing API",
    defaultEndpoint: "https://api.bing.microsoft.com/v7.0/search",
    endpointPlaceholder: "https://api.bing.microsoft.com/v7.0/search",
    apiKeyPlaceholder: "Bing subscription key",
  },
};

export function isSearchApiKind(value: string): value is WebSearchApiKind {
  return (SEARCH_API_KINDS as readonly string[]).includes(value);
}

export function isBuiltinSearchProviderId(id: string) {
  return isSearchApiKind(id);
}

export function getSearchKindMeta(kind: string): SearchKindMeta {
  if (isSearchApiKind(kind)) return SEARCH_KIND_META[kind];
  return SEARCH_KIND_META.tavily;
}

export function searchProviderDisplayName(
  provider: Pick<WebSearchProviderConfig, "id" | "kind" | "name">,
) {
  const named = provider.name?.trim();
  if (named) return named;
  return getSearchKindMeta(provider.kind).label;
}

export function makeSearchProviderId() {
  return `search-${Date.now().toString(36)}-${Math.random()
    .toString(36)
    .slice(2, 8)}`;
}

export function makeSearchProvider(
  kind: WebSearchApiKind,
  patch: Partial<WebSearchProviderConfig> = {},
): WebSearchProviderConfig {
  const meta = SEARCH_KIND_META[kind];
  const id = patch.id?.trim() || makeSearchProviderId();
  return {
    id,
    kind,
    name: patch.name?.trim() || meta.defaultName,
    api_key: patch.api_key ?? "",
    endpoint: patch.endpoint ?? "",
    enabled: patch.enabled !== false,
  };
}

/** Ensure builtin tavily/serper/bing rows exist; keep custom entries. */
export function normalizeSearchProviders(
  list: WebSearchProviderConfig[] | undefined | null,
): WebSearchProviderConfig[] {
  const incoming = list ?? [];
  const byId = new Map<string, WebSearchProviderConfig>();
  for (const item of incoming) {
    const id = item.id?.trim() || item.kind?.trim();
    if (!id) continue;
    const kind = isSearchApiKind(item.kind) ? item.kind : "tavily";
    byId.set(id, {
      id,
      kind,
      name: item.name?.trim() || (isBuiltinSearchProviderId(id) ? SEARCH_KIND_META[kind].defaultName : item.name),
      api_key: item.api_key ?? "",
      endpoint: item.endpoint ?? "",
      enabled: item.enabled !== false,
    });
  }

  const out: WebSearchProviderConfig[] = [];
  for (const kind of SEARCH_API_KINDS) {
    const existing = byId.get(kind);
    if (existing) {
      out.push({
        ...existing,
        id: kind,
        kind,
        name: existing.name?.trim() || SEARCH_KIND_META[kind].defaultName,
      });
      byId.delete(kind);
    } else {
      out.push({
        id: kind,
        kind,
        name: SEARCH_KIND_META[kind].defaultName,
        api_key: "",
        endpoint: "",
        enabled: true,
      });
    }
  }

  for (const item of byId.values()) {
    out.push(item);
  }
  return out;
}
