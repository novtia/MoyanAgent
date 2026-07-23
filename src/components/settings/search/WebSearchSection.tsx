import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";
import type { WebSearchProviderConfig } from "../../../types";

const API_KINDS = ["tavily", "serper", "bing"] as const;
type ApiKind = (typeof API_KINDS)[number];

const SAVE_DEBOUNCE_MS = 500;

interface ProviderDraft {
  api_key: string;
  endpoint: string;
}

function findProvider(
  list: WebSearchProviderConfig[],
  kind: string,
): WebSearchProviderConfig | undefined {
  return list.find((p) => p.kind === kind || p.id === kind);
}

export function WebSearchSection() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);

  const enabled = settings?.web_search_enabled ?? true;
  const backend = settings?.web_search_backend ?? "local";
  const localEngine = settings?.web_search_local_engine ?? "duckduckgo";
  const providers = settings?.web_search_providers ?? [];

  const [maxResults, setMaxResults] = useState(
    String(settings?.web_search_max_results ?? 5),
  );
  const [drafts, setDrafts] = useState<Record<string, ProviderDraft>>({});
  const [visibleKeys, setVisibleKeys] = useState<Record<string, boolean>>({});
  const timers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});

  useEffect(() => {
    setMaxResults(String(settings?.web_search_max_results ?? 5));
  }, [settings?.web_search_max_results]);

  useEffect(() => {
    const next: Record<string, ProviderDraft> = {};
    for (const kind of API_KINDS) {
      const p = findProvider(providers, kind);
      next[kind] = { api_key: p?.api_key ?? "", endpoint: p?.endpoint ?? "" };
    }
    setDrafts(next);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings?.web_search_providers]);

  useEffect(
    () => () => {
      Object.values(timers.current).forEach((tm) => clearTimeout(tm));
    },
    [],
  );

  const saveProvider = (kind: ApiKind, patch: Partial<WebSearchProviderConfig>) => {
    const list = [...(settings?.web_search_providers ?? [])];
    const idx = list.findIndex((p) => p.kind === kind || p.id === kind);
    const base: WebSearchProviderConfig =
      idx >= 0
        ? { ...list[idx] }
        : { id: kind, kind, api_key: "", endpoint: "", enabled: true };
    const merged = { ...base, ...patch };
    if (idx >= 0) list[idx] = merged;
    else list.push(merged);
    void update({ web_search_providers: list });
  };

  const scheduleSave = (key: string, fn: () => void) => {
    if (timers.current[key]) clearTimeout(timers.current[key]);
    timers.current[key] = setTimeout(fn, SAVE_DEBOUNCE_MS);
  };

  const onKeyChange = (kind: ApiKind, value: string) => {
    setDrafts((d) => ({ ...d, [kind]: { ...d[kind], api_key: value } }));
    scheduleSave(`${kind}:key`, () => saveProvider(kind, { api_key: value.trim() }));
  };

  const onEndpointChange = (kind: ApiKind, value: string) => {
    setDrafts((d) => ({ ...d, [kind]: { ...d[kind], endpoint: value } }));
    scheduleSave(`${kind}:endpoint`, () =>
      saveProvider(kind, { endpoint: value.trim() }),
    );
  };

  const onMaxResultsChange = (value: string) => {
    setMaxResults(value);
    const n = Number.parseInt(value, 10);
    if (Number.isFinite(n) && n >= 1) {
      scheduleSave("maxResults", () =>
        void update({ web_search_max_results: Math.min(n, 20) }),
      );
    }
  };

  if (!settings) return null;

  const backendOptions: { value: string; label: string }[] = [
    { value: "local", label: t("settings.search.backendLocal") },
    { value: "tavily", label: "Tavily" },
    { value: "serper", label: "Serper" },
    { value: "bing", label: "Bing API" },
  ];

  return (
    <div className="settings-stack">
      <div className="settings-card">
        <div className="settings-card-head">
          <div>
            <div className="settings-card-title">
              {t("settings.search.enableTitle")}
            </div>
            <div className="settings-card-desc">
              {t("settings.search.enableDesc")}
            </div>
          </div>
          <button
            type="button"
            className={`settings-toggle ${enabled ? "settings-toggle--on" : ""}`}
            role="switch"
            aria-checked={enabled}
            aria-label={t("settings.search.enableTitle")}
            onClick={() => void update({ web_search_enabled: !enabled })}
          >
            <span className="settings-toggle-thumb" />
          </button>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-card-head">
          <div>
            <div className="settings-card-title">
              {t("settings.search.backendTitle")}
            </div>
            <div className="settings-card-desc">
              {t("settings.search.backendDesc")}
            </div>
          </div>
          <select
            className="settings-select"
            aria-label={t("settings.search.backendTitle")}
            value={backend}
            onChange={(e) => void update({ web_search_backend: e.target.value })}
          >
            {backendOptions.map((opt) => (
              <option key={opt.value} value={opt.value}>
                {opt.label}
              </option>
            ))}
          </select>
        </div>

        {backend === "local" && (
          <div className="settings-card-head">
            <div>
              <div className="settings-card-title">
                {t("settings.search.localEngineTitle")}
              </div>
              <div className="settings-card-desc">
                {t("settings.search.localEngineDesc")}
              </div>
            </div>
            <select
              className="settings-select"
              aria-label={t("settings.search.localEngineTitle")}
              value={localEngine}
              onChange={(e) =>
                void update({ web_search_local_engine: e.target.value })
              }
            >
              <option value="duckduckgo">DuckDuckGo</option>
              <option value="bing">Bing</option>
            </select>
          </div>
        )}

        <div className="settings-card-head">
          <div>
            <div className="settings-card-title">
              {t("settings.search.maxResultsTitle")}
            </div>
            <div className="settings-card-desc">
              {t("settings.search.maxResultsDesc")}
            </div>
          </div>
          <input
            type="number"
            min={1}
            max={20}
            className="settings-select"
            style={{ width: 88 }}
            value={maxResults}
            onChange={(e) => onMaxResultsChange(e.target.value)}
          />
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-card-head">
          <div>
            <div className="settings-card-title">
              {t("settings.search.providersTitle")}
            </div>
            <div className="settings-card-desc">
              {t("settings.search.providersDesc")}
            </div>
          </div>
        </div>

        <div className="model-settings-form">
          {API_KINDS.map((kind) => {
            const draft = drafts[kind] ?? { api_key: "", endpoint: "" };
            const label = kind === "bing" ? "Bing API" : kind[0].toUpperCase() + kind.slice(1);
            const show = visibleKeys[kind] ?? false;
            return (
              <div className="row" key={kind}>
                <label className="field-label">
                  {label} · {t("settings.search.apiKeyLabel")}
                </label>
                <div style={{ display: "flex", gap: 8 }}>
                  <input
                    type={show ? "text" : "password"}
                    value={draft.api_key}
                    spellCheck={false}
                    placeholder={t("settings.search.apiKeyPlaceholder")}
                    style={{ flex: 1 }}
                    onChange={(e) => onKeyChange(kind, e.target.value)}
                  />
                  <button
                    type="button"
                    className="btn"
                    onClick={() =>
                      setVisibleKeys((v) => ({ ...v, [kind]: !show }))
                    }
                  >
                    {show ? t("settings.llm.keyHide") : t("settings.llm.keyShow")}
                  </button>
                </div>
                <input
                  type="text"
                  value={draft.endpoint}
                  spellCheck={false}
                  placeholder={t("settings.search.endpointPlaceholder")}
                  onChange={(e) => onEndpointChange(kind, e.target.value)}
                />
              </div>
            );
          })}
          <div className="hint">{t("settings.search.keyHint")}</div>
        </div>
      </div>
    </div>
  );
}
