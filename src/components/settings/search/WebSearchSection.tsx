import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";
import type { WebSearchProviderConfig } from "../../../types";
import { SettingsSelectDropdown } from "../SettingsSelectDropdown";

const SIDEBAR_KINDS = ["local", "tavily", "serper", "bing"] as const;
type SidebarKind = (typeof SIDEBAR_KINDS)[number];
type ApiKind = Exclude<SidebarKind, "local">;

const API_KINDS: ApiKind[] = ["tavily", "serper", "bing"];

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

function isSidebarKind(value: string): value is SidebarKind {
  return (SIDEBAR_KINDS as readonly string[]).includes(value);
}

function providerLabel(kind: SidebarKind, localLabel: string): string {
  if (kind === "local") return localLabel;
  if (kind === "bing") return "Bing API";
  return kind[0].toUpperCase() + kind.slice(1);
}

function providerGlyphLetter(kind: SidebarKind): string {
  if (kind === "local") return "L";
  if (kind === "bing") return "B";
  return kind[0].toUpperCase();
}

function ProviderGlyph({ kind }: { kind: SidebarKind }) {
  return (
    <span className="model-provider-avatar" aria-hidden>
      {providerGlyphLetter(kind)}
    </span>
  );
}

function SearchIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.7"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden
    >
      <circle cx="11" cy="11" r="7" />
      <line x1="21" y1="21" x2="16.65" y2="16.65" />
    </svg>
  );
}

export function WebSearchSection() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);

  const backend = settings?.web_search_backend ?? "local";
  const localEngine = settings?.web_search_local_engine ?? "duckduckgo";
  const providers = settings?.web_search_providers ?? [];
  const localLabel = t("settings.search.localProviderName");

  const [selectedKind, setSelectedKind] = useState<SidebarKind>(() =>
    isSidebarKind(backend) ? backend : "local",
  );
  const [providerSearch, setProviderSearch] = useState("");
  const [drafts, setDrafts] = useState<Record<string, ProviderDraft>>({});
  const [visibleKeys, setVisibleKeys] = useState<Record<string, boolean>>({});
  const timers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});

  useEffect(() => {
    if (isSidebarKind(backend)) setSelectedKind(backend);
  }, [backend]);

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

  const selectProvider = (kind: SidebarKind) => {
    setSelectedKind(kind);
    if (kind !== backend) {
      void update({ web_search_backend: kind });
    }
  };

  const localEngineOptions = useMemo(
    () => [
      { value: "duckduckgo", label: "DuckDuckGo" },
      { value: "bing", label: "Bing" },
    ],
    [],
  );

  const filteredKinds = useMemo(() => {
    const q = providerSearch.trim().toLowerCase();
    if (!q) return [...SIDEBAR_KINDS];
    return SIDEBAR_KINDS.filter((kind) => {
      const label = providerLabel(kind, localLabel).toLowerCase();
      return label.includes(q) || kind.includes(q);
    });
  }, [providerSearch, localLabel]);

  if (!settings) return null;

  const isLocal = selectedKind === "local";
  const draft = !isLocal
    ? (drafts[selectedKind] ?? { api_key: "", endpoint: "" })
    : { api_key: "", endpoint: "" };
  const showKey = !isLocal ? (visibleKeys[selectedKind] ?? false) : false;
  const selectedLabel = providerLabel(selectedKind, localLabel);

  return (
    <div className="model-service-card">
      <div className="model-service-layout">
        <aside className="model-provider-pane">
          <div className="model-provider-search-wrap">
            <SearchIcon />
            <input
              type="search"
              value={providerSearch}
              placeholder={t("settings.search.providerSearchPlaceholder")}
              onChange={(e) => setProviderSearch(e.target.value)}
            />
          </div>
          <div className="model-provider-list">
            {filteredKinds.map((kind) => {
              const isActive = kind === backend;
              const hasKey =
                kind !== "local" &&
                Boolean((drafts[kind]?.api_key ?? "").trim());
              const label = providerLabel(kind, localLabel);
              const secondary =
                kind === "local"
                  ? t("settings.search.localProviderTag")
                  : hasKey
                    ? t("settings.search.providerConfigured")
                    : kind;
              return (
                <div
                  key={kind}
                  className={`model-provider-item ${
                    kind === selectedKind ? "active" : ""
                  }`}
                >
                  <button
                    type="button"
                    className="model-provider-item-body"
                    onClick={() => selectProvider(kind)}
                  >
                    <span
                      className={`model-provider-avatar ${
                        kind === "local" || hasKey
                          ? "web-search-avatar--configured"
                          : ""
                      }`}
                      aria-hidden
                    >
                      {providerGlyphLetter(kind)}
                    </span>
                    <span className="model-provider-name">
                      <span className="model-provider-name-text">{label}</span>
                      <span className="model-provider-sdk">
                        {isActive
                          ? t("settings.search.providerActive")
                          : secondary}
                      </span>
                    </span>
                  </button>
                </div>
              );
            })}
            {filteredKinds.length === 0 && (
              <div className="model-provider-empty">
                {t("settings.search.providerSearchEmpty")}
              </div>
            )}
          </div>
        </aside>

        <section className="model-provider-detail">
          <div className="model-provider-detail-inner">
            <div className="model-provider-hero">
              <ProviderGlyph kind={selectedKind} />
              <div className="model-provider-hero-text">
                <span className="model-provider-hero-name">{selectedLabel}</span>
                <div className="model-provider-hero-meta">
                  <span className="model-provider-hero-sdk">
                    {isLocal ? t("settings.search.localProviderTag") : selectedKind}
                  </span>
                </div>
              </div>
            </div>

            {isLocal ? (
              <div className="model-provider-config">
                <div className="model-provider-fields">
                  <div className="web-search-local-engine-row">
                    <div className="settings-row-main">
                      <div className="settings-row-title">
                        {t("settings.search.localEngineTitle")}
                      </div>
                      <div className="settings-row-desc">
                        {t("settings.search.localEngineDesc")}
                      </div>
                    </div>
                    <div className="settings-row-control">
                      <SettingsSelectDropdown
                        ariaLabel={t("settings.search.localEngineTitle")}
                        value={localEngine}
                        options={localEngineOptions}
                        onChange={(value) =>
                          void update({ web_search_local_engine: value })
                        }
                      />
                    </div>
                  </div>
                  <div className="hint">{t("settings.search.localProviderHint")}</div>
                </div>
              </div>
            ) : (
              <>
                <div className="web-search-section-head">
                  <div className="settings-row-title">
                    {t("settings.search.providersTitle")}
                  </div>
                  <div className="settings-row-desc">
                    {t("settings.search.providersDesc")}
                  </div>
                </div>

                <div className="model-provider-config">
                  <div className="model-provider-fields">
                    <div className="row">
                      <label className="field-label">
                        {t("settings.search.apiKeyLabel")}
                      </label>
                      <div className="input-affix">
                        <input
                          type={showKey ? "text" : "password"}
                          value={draft.api_key}
                          spellCheck={false}
                          autoComplete="off"
                          placeholder={t("settings.search.apiKeyPlaceholder")}
                          onChange={(e) =>
                            onKeyChange(selectedKind, e.target.value)
                          }
                        />
                        <button
                          type="button"
                          className="affix-btn"
                          onClick={() =>
                            setVisibleKeys((v) => ({
                              ...v,
                              [selectedKind]: !showKey,
                            }))
                          }
                        >
                          {showKey
                            ? t("settings.llm.keyHide")
                            : t("settings.llm.keyShow")}
                        </button>
                      </div>
                    </div>
                    <div className="row">
                      <input
                        type="text"
                        value={draft.endpoint}
                        spellCheck={false}
                        placeholder={t("settings.search.endpointPlaceholder")}
                        onChange={(e) =>
                          onEndpointChange(selectedKind, e.target.value)
                        }
                      />
                    </div>
                    <div className="hint">{t("settings.search.keyHint")}</div>
                  </div>
                </div>
              </>
            )}
          </div>
        </section>
      </div>
    </div>
  );
}
