import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";
import type { WebSearchApiKind, WebSearchProviderConfig } from "../../../types";
import { dialog } from "../../ui";
import { SettingsSelectDropdown } from "../SettingsSelectDropdown";
import { SearchBrandIcon } from "./SearchBrandIcon";
import {
  SEARCH_API_KINDS,
  SEARCH_KIND_META,
  getSearchKindMeta,
  isBuiltinSearchProviderId,
  isSearchApiKind,
  makeSearchProvider,
  normalizeSearchProviders,
  searchProviderDisplayName,
} from "./searchProviders";

const SAVE_DEBOUNCE_MS = 500;

interface ProviderDraft {
  name: string;
  api_key: string;
  endpoint: string;
}

type SidebarId = "local" | string;

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

function PlusIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" aria-hidden>
      <path d="M12 5v14M5 12h14" strokeLinecap="round" />
    </svg>
  );
}

export function WebSearchSection() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);

  const backend = settings?.web_search_backend ?? "local";
  const localEngine = settings?.web_search_local_engine ?? "duckduckgo";
  const localLabel = t("settings.search.localProviderName");

  const providers = useMemo(
    () => normalizeSearchProviders(settings?.web_search_providers),
    [settings?.web_search_providers],
  );

  const [selectedId, setSelectedId] = useState<SidebarId>(() =>
    backend === "local" || !backend ? "local" : backend,
  );
  const [providerSearch, setProviderSearch] = useState("");
  const [drafts, setDrafts] = useState<Record<string, ProviderDraft>>({});
  const [visibleKeys, setVisibleKeys] = useState<Record<string, boolean>>({});
  const [adding, setAdding] = useState(false);
  const timers = useRef<Record<string, ReturnType<typeof setTimeout>>>({});
  const seededRef = useRef(false);

  // Seed builtin rows into settings once if missing.
  useEffect(() => {
    if (!settings || seededRef.current) return;
    const raw = settings.web_search_providers ?? [];
    const normalized = normalizeSearchProviders(raw);
    const needsSeed =
      SEARCH_API_KINDS.some((kind) => !raw.some((p) => p.id === kind || p.kind === kind)) ||
      normalized.length !== raw.length;
    seededRef.current = true;
    if (needsSeed) {
      void update({ web_search_providers: normalized });
    }
  }, [settings, update]);

  useEffect(() => {
    if (backend === "local" || !backend) {
      setSelectedId("local");
      return;
    }
    if (providers.some((p) => p.id === backend || p.kind === backend)) {
      const match = providers.find((p) => p.id === backend) ?? providers.find((p) => p.kind === backend);
      if (match) setSelectedId(match.id);
    }
  }, [backend, providers]);

  useEffect(() => {
    const next: Record<string, ProviderDraft> = {};
    for (const p of providers) {
      next[p.id] = {
        name: searchProviderDisplayName(p),
        api_key: p.api_key ?? "",
        endpoint: p.endpoint ?? "",
      };
    }
    setDrafts(next);
  }, [providers]);

  useEffect(
    () => () => {
      Object.values(timers.current).forEach((tm) => clearTimeout(tm));
    },
    [],
  );

  const persistProviders = (list: WebSearchProviderConfig[]) =>
    update({ web_search_providers: normalizeSearchProviders(list) });

  const saveProvider = (id: string, patch: Partial<WebSearchProviderConfig>) => {
    const list = [...providers];
    const idx = list.findIndex((p) => p.id === id);
    if (idx < 0) return;
    list[idx] = { ...list[idx], ...patch };
    void persistProviders(list);
  };

  const scheduleSave = (key: string, fn: () => void) => {
    if (timers.current[key]) clearTimeout(timers.current[key]);
    timers.current[key] = setTimeout(fn, SAVE_DEBOUNCE_MS);
  };

  const selectProvider = (id: SidebarId) => {
    setSelectedId(id);
    if (id !== backend) {
      void update({ web_search_backend: id });
    }
  };

  const addProvider = async (draft: { name: string; kind: WebSearchApiKind }) => {
    const created = makeSearchProvider(draft.kind, { name: draft.name });
    const next = [...providers, created];
    await persistProviders(next);
    setAdding(false);
    setSelectedId(created.id);
    await update({ web_search_backend: created.id });
  };

  const removeProvider = async (id: string) => {
    if (isBuiltinSearchProviderId(id)) return;
    const label =
      drafts[id]?.name ||
      searchProviderDisplayName(providers.find((p) => p.id === id) ?? { id, kind: "tavily" });
    const ok = await dialog.confirm(
      t("settings.search.deleteConfirm", { name: label }),
      { type: "danger", confirmLabel: t("settings.search.deleteAction") },
    );
    if (!ok) return;
    const next = providers.filter((p) => p.id !== id);
    await persistProviders(next);
    if (selectedId === id || backend === id) {
      setSelectedId("local");
      await update({ web_search_backend: "local" });
    }
  };

  const localEngineOptions = useMemo(
    () => [
      { value: "duckduckgo", label: "DuckDuckGo" },
      { value: "bing", label: "Bing" },
    ],
    [],
  );

  const filteredProviders = useMemo(() => {
    const q = providerSearch.trim().toLowerCase();
    const items: { id: SidebarId; kind: string; label: string; secondary: string; configured: boolean; custom: boolean }[] =
      [
        {
          id: "local",
          kind: localEngine === "bing" ? "bing" : "local",
          label: localLabel,
          secondary: t("settings.search.localProviderTag"),
          configured: true,
          custom: false,
        },
        ...providers.map((p) => {
          const hasKey = Boolean((drafts[p.id]?.api_key ?? p.api_key ?? "").trim());
          return {
            id: p.id,
            kind: p.kind,
            label: drafts[p.id]?.name || searchProviderDisplayName(p),
            secondary: hasKey
              ? t("settings.search.providerConfigured")
              : getSearchKindMeta(p.kind).label,
            configured: hasKey,
            custom: !isBuiltinSearchProviderId(p.id),
          };
        }),
      ];
    if (!q) return items;
    return items.filter(
      (item) =>
        item.label.toLowerCase().includes(q) ||
        item.kind.toLowerCase().includes(q) ||
        item.id.toLowerCase().includes(q),
    );
  }, [providerSearch, localLabel, localEngine, providers, drafts, t]);

  if (!settings) return null;

  const isLocal = selectedId === "local";
  const selectedProvider = !isLocal
    ? providers.find((p) => p.id === selectedId)
    : undefined;
  const draft = selectedProvider
    ? (drafts[selectedProvider.id] ?? {
        name: searchProviderDisplayName(selectedProvider),
        api_key: "",
        endpoint: "",
      })
    : { name: "", api_key: "", endpoint: "" };
  const showKey = selectedProvider ? (visibleKeys[selectedProvider.id] ?? false) : false;
  const selectedKind = isLocal
    ? localEngine === "bing"
      ? "bing"
      : "local"
    : (selectedProvider?.kind ?? "tavily");
  const selectedLabel = isLocal
    ? localLabel
    : draft.name || (selectedProvider ? searchProviderDisplayName(selectedProvider) : "");
  const isCustom = !!selectedProvider && !isBuiltinSearchProviderId(selectedProvider.id);
  const kindMeta = selectedProvider
    ? getSearchKindMeta(selectedProvider.kind)
    : null;

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
            {filteredProviders.map((item) => {
              const isActive = item.id === backend;
              return (
                <div
                  key={item.id}
                  className={`model-provider-item ${
                    item.id === selectedId ? "active" : ""
                  }`}
                >
                  <button
                    type="button"
                    className="model-provider-item-body"
                    onClick={() => selectProvider(item.id)}
                  >
                    <SearchBrandIcon
                      kind={item.kind}
                      className={`model-provider-avatar ${
                        item.configured ? "web-search-avatar--configured" : ""
                      }`}
                      size={20}
                      fallback={item.label.charAt(0).toUpperCase()}
                    />
                    <span className="model-provider-name">
                      <span className="model-provider-name-text">{item.label}</span>
                      <span className="model-provider-sdk">
                        {isActive
                          ? t("settings.search.providerActive")
                          : item.secondary}
                      </span>
                    </span>
                  </button>
                </div>
              );
            })}
            {filteredProviders.length === 0 && (
              <div className="model-provider-empty">
                {t("settings.search.providerSearchEmpty")}
              </div>
            )}
          </div>
          <button
            type="button"
            className="btn model-provider-add"
            onClick={() => setAdding(true)}
          >
            <PlusIcon />
            <span>{t("settings.search.addProvider")}</span>
          </button>
        </aside>

        <section className="model-provider-detail">
          <div className="model-provider-detail-inner">
            <div className="model-provider-hero">
              <SearchBrandIcon
                kind={selectedKind}
                className="model-provider-avatar"
                size={22}
                fallback={selectedLabel.charAt(0).toUpperCase() || "·"}
              />
              <div className="model-provider-hero-text">
                <span className="model-provider-hero-name">{selectedLabel}</span>
                <div className="model-provider-hero-meta">
                  <span className="model-provider-hero-sdk">
                    {isLocal
                      ? t("settings.search.localProviderTag")
                      : kindMeta?.label ?? selectedKind}
                  </span>
                </div>
              </div>
              {isCustom && selectedProvider && (
                <button
                  type="button"
                  className="btn danger web-search-delete-btn"
                  onClick={() => void removeProvider(selectedProvider.id)}
                >
                  {t("settings.search.deleteAction")}
                </button>
              )}
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
            ) : selectedProvider ? (
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
                        {t("settings.search.nameLabel")}
                      </label>
                      <input
                        type="text"
                        value={draft.name}
                        spellCheck={false}
                        onChange={(e) => {
                          const value = e.target.value;
                          const id = selectedProvider.id;
                          setDrafts((d) => ({
                            ...d,
                            [id]: { ...d[id], name: value },
                          }));
                          scheduleSave(`${id}:name`, () =>
                            saveProvider(id, { name: value.trim() }),
                          );
                        }}
                      />
                    </div>
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
                          placeholder={kindMeta?.apiKeyPlaceholder}
                          onChange={(e) => {
                            const value = e.target.value;
                            const id = selectedProvider.id;
                            setDrafts((d) => ({
                              ...d,
                              [id]: { ...d[id], api_key: value },
                            }));
                            scheduleSave(`${id}:key`, () =>
                              saveProvider(id, { api_key: value.trim() }),
                            );
                          }}
                        />
                        <button
                          type="button"
                          className="affix-btn"
                          onClick={() =>
                            setVisibleKeys((v) => ({
                              ...v,
                              [selectedProvider.id]: !showKey,
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
                      <label className="field-label">
                        {t("settings.search.endpointLabel")}
                      </label>
                      <input
                        type="text"
                        value={draft.endpoint}
                        spellCheck={false}
                        placeholder={
                          kindMeta?.endpointPlaceholder ||
                          t("settings.search.endpointPlaceholder")
                        }
                        onChange={(e) => {
                          const value = e.target.value;
                          const id = selectedProvider.id;
                          setDrafts((d) => ({
                            ...d,
                            [id]: { ...d[id], endpoint: value },
                          }));
                          scheduleSave(`${id}:endpoint`, () =>
                            saveProvider(id, { endpoint: value.trim() }),
                          );
                        }}
                      />
                    </div>
                    <div className="hint">{t("settings.search.keyHint")}</div>
                  </div>
                </div>
              </>
            ) : null}
          </div>
        </section>
      </div>

      {adding && (
        <AddSearchProviderModal
          onClose={() => setAdding(false)}
          onAdd={addProvider}
        />
      )}
    </div>
  );
}

function AddSearchProviderModal({
  onClose,
  onAdd,
}: {
  onClose: () => void;
  onAdd: (draft: { name: string; kind: WebSearchApiKind }) => void | Promise<void>;
}) {
  const { t } = useTranslation();
  const [kind, setKind] = useState<WebSearchApiKind>("tavily");
  const [name, setName] = useState(SEARCH_KIND_META.tavily.defaultName);
  const [nameTouched, setNameTouched] = useState(false);

  const meta = SEARCH_KIND_META[kind];
  const canSubmit = !!name.trim() && isSearchApiKind(kind);

  const changeKind = (next: string) => {
    if (!isSearchApiKind(next)) return;
    setKind(next);
    if (!nameTouched) setName(SEARCH_KIND_META[next].defaultName);
  };

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div
        className="modal model-settings-modal add-provider-modal"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>{t("settings.search.addProviderTitle")}</h3>
          <button type="button" className="close" onClick={onClose}>
            {t("settings.search.close")}
          </button>
        </div>
        <div className="modal-body">
          <div className="model-settings-form">
            <div className="provider-avatar-preview">
              <SearchBrandIcon
                kind={kind}
                className="provider-avatar-preview-image"
                size={22}
                fallback={meta.label.charAt(0)}
              />
              <div>
                <strong>{name.trim() || meta.defaultName}</strong>
                <em>{meta.label}</em>
              </div>
            </div>
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span>{" "}
                {t("settings.search.nameLabel")}
              </label>
              <input
                type="text"
                value={name}
                autoFocus
                onChange={(e) => {
                  setNameTouched(true);
                  setName(e.target.value);
                }}
              />
            </div>
            <div className="row">
              <label className="field-label">
                <span className="required-star">*</span>{" "}
                {t("settings.search.kindLabel")}
              </label>
              <select value={kind} onChange={(e) => changeKind(e.target.value)}>
                {SEARCH_API_KINDS.map((id) => (
                  <option key={id} value={id}>
                    {SEARCH_KIND_META[id].label}
                  </option>
                ))}
              </select>
              <div className="hint">{meta.description}</div>
            </div>
          </div>
        </div>
        <div className="modal-foot">
          <button type="button" className="btn" onClick={onClose}>
            {t("settings.search.cancel")}
          </button>
          <button
            type="button"
            className="btn primary"
            disabled={!canSubmit}
            onClick={() => {
              if (!canSubmit) return;
              void onAdd({ name: name.trim(), kind });
            }}
          >
            {t("settings.search.addAction")}
          </button>
        </div>
      </div>
    </div>
  );
}
