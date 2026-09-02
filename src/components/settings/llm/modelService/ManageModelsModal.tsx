import { useEffect, useMemo, useState } from "react";
import type {
  ModelPricing,
  ModelServiceModel,
  RemoteModelInfo,
} from "../../../../types";
import { api } from "../../../../api/tauri";
import {
  groupFromModelId,
  manageGroupLabel,
  manageGroupMark,
  normalizeProviderSdk,
  resolveManageGroupIconId,
  resolveModelBrandIconId,
  shortModelName,
  type ManageModelsFilter,
} from "../modelServices";
import { ProviderBrandIcon } from "../ProviderBrandIcon";
import { MinusIcon, PlusIcon, SearchIcon } from "./icons";

interface ManageModelsModalProps {
  providerName: string;
  sdk?: string;
  endpoint: string;
  apiKey: string;
  localModels: ModelServiceModel[];
  onAdd: (info: RemoteModelInfo | string) => void | Promise<void>;
  onRemove: (modelId: string) => void | Promise<void>;
  onClose: () => void;
}

type ManageModelEntry = {
  id: string;
  inLocal: boolean;
  group: string;
  name?: string | null;
  context_window?: number | null;
  pricing?: ModelPricing | null;
  remote?: RemoteModelInfo | null;
};

type ManageGroupNav = {
  id: string;
  count: number;
  hasLocal: boolean;
};

function formatTokenCount(n: number): string {
  if (n >= 1_000_000) {
    const m = n / 1_000_000;
    return `${Number.isInteger(m) ? m : m.toFixed(1)}M ctx`;
  }
  if (n >= 1000) {
    const k = n / 1000;
    return `${Number.isInteger(k) ? k : k.toFixed(0)}K ctx`;
  }
  return `${n} ctx`;
}

function formatPriceShort(n: number): string {
  if (n >= 100) return n.toFixed(0);
  if (n >= 10) return n.toFixed(1);
  if (n >= 1) return n.toFixed(2);
  return n.toFixed(3);
}

function formatManageMeta(entry: ManageModelEntry): string {
  const parts: string[] = [];
  if (entry.context_window != null && entry.context_window > 0) {
    parts.push(formatTokenCount(entry.context_window));
  }
  const p = entry.pricing;
  if (p) {
    const inP = p.inputPer1M;
    const outP = p.outputPer1M;
    if (inP != null || outP != null) {
      const a = inP != null ? formatPriceShort(inP) : "—";
      const b = outP != null ? formatPriceShort(outP) : "—";
      parts.push(`${a}/${b}`);
    }
  }
  return parts.join(" · ");
}

export function ManageModelsModal({
  providerName,
  sdk,
  endpoint,
  apiKey,
  localModels,
  onAdd,
  onRemove,
  onClose,
}: ManageModelsModalProps) {
  const [remoteModels, setRemoteModels] = useState<RemoteModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [activeFilter, setActiveFilter] = useState<ManageModelsFilter>("all");

  const load = () => {
    setLoading(true);
    setError(null);
    let cancelled = false;
    void api
      .fetchProviderModels(normalizeProviderSdk(sdk), endpoint, apiKey)
      .then((list) => {
        if (!cancelled) setRemoteModels(list);
      })
      .catch((err) => {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  };

  useEffect(() => {
    const cleanup = load();
    return cleanup;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const remoteById = useMemo(() => {
    const map = new Map<string, RemoteModelInfo>();
    for (const m of remoteModels) map.set(m.id, m);
    return map;
  }, [remoteModels]);

  const entries = useMemo(() => {
    const list: ManageModelEntry[] = [];
    const seen = new Set<string>();
    for (const model of localModels) {
      if (seen.has(model.id)) continue;
      seen.add(model.id);
      const remote = remoteById.get(model.id) ?? null;
      list.push({
        id: model.id,
        inLocal: true,
        group: model.group || groupFromModelId(model.id),
        name: model.name,
        context_window: model.context_window ?? remote?.context_window ?? null,
        pricing: model.pricing ?? remote?.pricing ?? null,
        remote,
      });
    }
    for (const remote of remoteModels) {
      if (seen.has(remote.id)) continue;
      seen.add(remote.id);
      list.push({
        id: remote.id,
        inLocal: false,
        group: groupFromModelId(remote.id),
        name: remote.name,
        context_window: remote.context_window ?? null,
        pricing: remote.pricing ?? null,
        remote,
      });
    }
    return list;
  }, [localModels, remoteModels, remoteById]);

  const groups = useMemo(() => {
    const map = new Map<string, ManageGroupNav>();
    for (const entry of entries) {
      const cur = map.get(entry.group) ?? {
        id: entry.group,
        count: 0,
        hasLocal: false,
      };
      cur.count += 1;
      if (entry.inLocal) cur.hasLocal = true;
      map.set(entry.group, cur);
    }
    return Array.from(map.values()).sort((a, b) => {
      if (a.id === "custom") return 1;
      if (b.id === "custom") return -1;
      if (b.count !== a.count) return b.count - a.count;
      return a.id.localeCompare(b.id);
    });
  }, [entries]);

  useEffect(() => {
    if (activeFilter === "all" || activeFilter === "added") return;
    if (!groups.some((g) => g.id === activeFilter)) {
      setActiveFilter("all");
    }
  }, [groups, activeFilter]);

  const addedTotal = useMemo(
    () => entries.filter((e) => e.inLocal).length,
    [entries],
  );

  const filteredEntries = useMemo(() => {
    const q = search.trim().toLowerCase();
    return entries.filter((entry) => {
      if (activeFilter === "added" && !entry.inLocal) return false;
      if (
        activeFilter !== "all" &&
        activeFilter !== "added" &&
        entry.group !== activeFilter
      ) {
        return false;
      }
      if (!q) return true;
      const name = (entry.name || shortModelName(entry.id)).toLowerCase();
      return entry.id.toLowerCase().includes(q) || name.includes(q);
    });
  }, [entries, activeFilter, search]);

  const filteredAdded = useMemo(
    () => filteredEntries.filter((e) => e.inLocal).length,
    [filteredEntries],
  );

  const toggle = async (entry: ManageModelEntry) => {
    setBusyId(entry.id);
    try {
      if (entry.inLocal) {
        await onRemove(entry.id);
      } else {
        await onAdd(entry.remote ?? entry.id);
      }
    } finally {
      setBusyId(null);
    }
  };

  const filterTitle =
    activeFilter === "all"
      ? "全部模型"
      : activeFilter === "added"
        ? "已添加"
        : manageGroupLabel(activeFilter);

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div
        className="modal model-settings-modal manage-models-modal"
        onMouseDown={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h3>
            管理模型
            <span className="manage-models-provider-tag">{providerName}</span>
          </h3>
          <button type="button" className="close" onClick={onClose}>
            关闭
          </button>
        </div>
        <div className="modal-body">
          <div className="manage-models-body">
            <aside className="manage-models-sidebar">
              <div className="manage-models-sidebar-head">浏览</div>
              <nav className="manage-models-nav">
                <div className="manage-models-nav-section">
                  <button
                    type="button"
                    className={`manage-models-nav-item ${
                      activeFilter === "all" ? "active" : ""
                    }`}
                    onClick={() => setActiveFilter("all")}
                  >
                    <span className="manage-models-nav-icon is-all">全</span>
                    <span className="manage-models-nav-label">全部</span>
                    <span className="manage-models-nav-count">
                      {entries.length}
                    </span>
                  </button>
                  <button
                    type="button"
                    className={`manage-models-nav-item ${
                      activeFilter === "added" ? "active" : ""
                    } ${addedTotal > 0 ? "has-added" : ""}`}
                    onClick={() => setActiveFilter("added")}
                  >
                    <span className="manage-models-nav-icon is-added">✓</span>
                    <span className="manage-models-nav-label">已添加</span>
                    <span className="manage-models-nav-count">{addedTotal}</span>
                  </button>
                </div>
                {groups.length > 0 && (
                  <div className="manage-models-nav-section">
                    <div className="manage-models-nav-section-title">分组</div>
                    {groups.map((group) => {
                      const hasBrand = !!resolveManageGroupIconId(group.id);
                      return (
                        <button
                          key={group.id}
                          type="button"
                          className={`manage-models-nav-item ${
                            activeFilter === group.id ? "active" : ""
                          } ${group.hasLocal ? "has-added" : ""}`}
                          onClick={() => setActiveFilter(group.id)}
                        >
                          <ProviderBrandIcon
                            className={`manage-models-nav-icon ${
                              group.id === "custom"
                                ? "is-other"
                                : hasBrand
                                  ? "is-brand"
                                  : "is-group"
                            }`}
                            group={group.id}
                            fallback={manageGroupMark(group.id)}
                            size={14}
                          />
                          <span className="manage-models-nav-label">
                            {manageGroupLabel(group.id)}
                          </span>
                          <span className="manage-models-nav-count">
                            {group.count}
                          </span>
                        </button>
                      );
                    })}
                  </div>
                )}
              </nav>
              <div className="manage-models-sidebar-foot">
                按模型 ID 前缀自动分组（如 openai/…）。
              </div>
            </aside>

            <div className="manage-models-main">
              <div className="manage-models-toolbar">
                <div className="manage-models-search">
                  <SearchIcon />
                  <input
                    type="search"
                    value={search}
                    placeholder="搜索模型 ID..."
                    autoFocus
                    spellCheck={false}
                    onChange={(e) => setSearch(e.target.value)}
                  />
                </div>
                <button
                  type="button"
                  className="btn manage-models-refresh"
                  disabled={loading}
                  title="刷新模型列表"
                  onClick={() => load()}
                >
                  {loading ? "拉取中…" : "刷新"}
                </button>
              </div>

              <div className="manage-models-statsbar">
                <span className="manage-models-stats-filter">{filterTitle}</span>
                <span className="manage-models-stats-sep">·</span>
                <span>
                  共 <b>{filteredEntries.length}</b> 个
                </span>
                <span className="manage-models-stats-sep">·</span>
                <span>
                  已添加 <b className="accent">{filteredAdded}</b> 个
                </span>
              </div>

              <div className="manage-models-content">
                {loading ? (
                  <div className="model-empty-state model-empty-state--center">
                    正在从供应商拉取模型列表…
                  </div>
                ) : error ? (
                  <div className="manage-models-error">
                    <div className="hint is-error">{error}</div>
                    <button type="button" className="btn" onClick={() => load()}>
                      重试
                    </button>
                  </div>
                ) : filteredEntries.length === 0 ? (
                  <div className="model-empty-state model-empty-state--center">
                    {search.trim() || activeFilter !== "all"
                      ? "没有匹配的模型。"
                      : "没有可用的模型。"}
                  </div>
                ) : (
                  <div className="manage-models-list">
                    {filteredEntries.map((entry) => {
                      const hasBrand = !!resolveModelBrandIconId(entry.id);
                      const meta = formatManageMeta(entry);
                      return (
                      <div
                        key={entry.id}
                        className={`manage-models-row ${
                          entry.inLocal ? "is-added" : ""
                        }`}
                      >
                        <ProviderBrandIcon
                          className={`manage-models-row-icon ${
                            hasBrand ? "has-brand" : ""
                          }`}
                          model={entry.id}
                          fallback={manageGroupMark(entry.group)}
                          size={15}
                        />
                        <div className="manage-models-row-text">
                          <span className="manage-models-name">
                            {entry.name?.trim() || shortModelName(entry.id)}
                          </span>
                          <span className="manage-models-id" title={entry.id}>
                            {entry.id}
                          </span>
                          {meta ? (
                            <span className="manage-models-meta">{meta}</span>
                          ) : null}
                        </div>
                        <div className="manage-models-row-actions">
                          {entry.inLocal && (
                            <span className="manage-models-tag">已添加</span>
                          )}
                          <button
                            type="button"
                            className={`manage-models-toggle ${
                              entry.inLocal ? "remove" : "add"
                            }`}
                            disabled={busyId === entry.id}
                            title={entry.inLocal ? "从本地删除" : "添加到本地"}
                            onClick={() => toggle(entry)}
                          >
                            {entry.inLocal ? <MinusIcon /> : <PlusIcon />}
                          </button>
                        </div>
                      </div>
                      );
                    })}
                  </div>
                )}
              </div>
            </div>
          </div>
        </div>
        <div className="modal-foot">
          <div className="manage-models-count">
            点击 + 添加模型，点击 − 移除模型 · 已添加 {addedTotal} / 共{" "}
            {entries.length}
          </div>
          <button type="button" className="btn primary" onClick={onClose}>
            完成
          </button>
        </div>
      </div>
    </div>
  );
}
