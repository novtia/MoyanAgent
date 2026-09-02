import { useEffect, useMemo, useState } from "react";
import type { RemoteModelEndpoint } from "../../../../types";
import { api } from "../../../../api/tauri";
import {
  isOpenRouterEndpoint,
  normalizeRouteProviders,
  resolveBrandIconId,
} from "../modelServices";
import { ProviderBrandIcon } from "../ProviderBrandIcon";

function useModelEndpoints(
  endpoint: string,
  apiKey: string,
  modelId: string,
  enabled: boolean,
) {
  const [endpoints, setEndpoints] = useState<RemoteModelEndpoint[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!enabled || !modelId.trim()) {
      setEndpoints([]);
      setError(null);
      setLoading(false);
      return;
    }
    let cancelled = false;
    const timer = window.setTimeout(() => {
      setLoading(true);
      setError(null);
      void api
        .fetchModelEndpoints(endpoint, apiKey, modelId.trim())
        .then((list) => {
          if (!cancelled) setEndpoints(list);
        })
        .catch((err) => {
          if (!cancelled) {
            setEndpoints([]);
            setError(err instanceof Error ? err.message : String(err));
          }
        })
        .finally(() => {
          if (!cancelled) setLoading(false);
        });
    }, 350);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [enabled, endpoint, apiKey, modelId]);

  return { endpoints, loading, error };
}

export function RouteProvidersField({
  endpoint,
  apiKey,
  modelId,
  selected,
  onChange,
}: {
  endpoint: string;
  apiKey: string;
  modelId: string;
  selected: string[];
  onChange: (slugs: string[]) => void;
}) {
  const show =
    isOpenRouterEndpoint(endpoint) || selected.length > 0;
  const { endpoints, loading, error } = useModelEndpoints(
    endpoint,
    apiKey,
    modelId,
    isOpenRouterEndpoint(endpoint) && !!modelId.trim(),
  );
  const [customSlug, setCustomSlug] = useState("");

  const options = useMemo(() => {
    const map = new Map<string, RemoteModelEndpoint>();
    for (const ep of endpoints) map.set(ep.slug.toLowerCase(), ep);
    for (const slug of selected) {
      const key = slug.toLowerCase();
      if (!map.has(key)) map.set(key, { slug, name: slug });
    }
    return Array.from(map.values());
  }, [endpoints, selected]);

  if (!show) return null;

  const addCustom = () => {
    const next = normalizeRouteProviders([...selected, customSlug]);
    if (next.length === selected.length) {
      setCustomSlug("");
      return;
    }
    onChange(next);
    setCustomSlug("");
  };

  return (
    <div className="model-modal-section">
      <div className="model-modal-section-title">路由供应商</div>
      <div className="hint model-pricing-hint">
        OpenRouter 的 provider 参数。留空则自动在可用上游间负载均衡；选中后请求只会发给这些供应商。
      </div>
      <div className="model-capability-row">
        <button
          type="button"
          className={`model-capability-chip ${selected.length === 0 ? "active" : ""}`}
          onClick={() => onChange([])}
        >
          自动
        </button>
        {options.map((ep) => {
          const active = selected.some(
            (s) => s.toLowerCase() === ep.slug.toLowerCase(),
          );
          return (
            <button
              key={ep.slug}
              type="button"
              className={`model-capability-chip model-route-chip ${active ? "active" : ""}`}
              title={ep.slug}
              onClick={() =>
                onChange(
                  active
                    ? selected.filter(
                        (s) => s.toLowerCase() !== ep.slug.toLowerCase(),
                      )
                    : normalizeRouteProviders([...selected, ep.slug]),
                )
              }
            >
              {resolveBrandIconId(ep.slug) ? (
                <ProviderBrandIcon
                  className="model-route-chip-icon"
                  provider={ep.slug}
                  size={14}
                />
              ) : null}
              <span>{ep.name}</span>
            </button>
          );
        })}
      </div>
      {loading && <div className="hint">正在拉取该模型的可用上游…</div>}
      {!loading && error && (
        <div className="hint">无法拉取上游列表，可手动输入 slug 添加。</div>
      )}
      <div className="row model-route-custom">
        <input
          type="text"
          value={customSlug}
          spellCheck={false}
          placeholder="输入供应商 slug，回车添加，如 alibaba"
          onChange={(e) => setCustomSlug(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              addCustom();
            }
          }}
        />
      </div>
    </div>
  );
}
