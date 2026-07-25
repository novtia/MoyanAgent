import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TokenUsageEventRow } from "../../types";
import { formatInt, formatTimeOfDay } from "./format";

const PAGE_SIZE = 50;

interface EventsTableProps {
  events: TokenUsageEventRow[];
  page: number;
  hasMore: boolean;
  models: string[];
  eventKind: string | null;
  model: string | null;
  onEventKindChange: (kind: string | null) => void;
  onModelChange: (model: string | null) => void;
  onPageChange: (page: number) => void;
  loading?: boolean;
}

export function EventsTable({
  events,
  page,
  hasMore,
  models,
  eventKind,
  model,
  onEventKindChange,
  onModelChange,
  onPageChange,
  loading,
}: EventsTableProps) {
  const { t } = useTranslation();
  const [kindOpen, setKindOpen] = useState(false);
  const [modelOpen, setModelOpen] = useState(false);

  const kindLabel = eventKind
    ? eventKind.replace("_", " ")
    : t("usage.allKinds");
  const modelLabel = model ?? t("usage.allModels");

  const approxTotal = useMemo(() => {
    // We don't have a total count API; show "at least" based on page fullness.
    const known = page * PAGE_SIZE + events.length;
    return hasMore ? `${known}+` : String(known);
  }, [page, events.length, hasMore]);

  return (
    <div className="usage-card">
      <div className="usage-card-head">
        <span className="t">{t("usage.eventsTitle")}</span>
        <span className="m">{t("usage.eventsMeta", { size: PAGE_SIZE })}</span>
        <div className="usage-filter-wrap">
          <button
            type="button"
            className={`usage-fchip ${eventKind ? "on" : ""}`}
            onClick={() => {
              setKindOpen((v) => !v);
              setModelOpen(false);
            }}
          >
            {kindLabel}
            <Chevron />
          </button>
          {kindOpen && (
            <div className="usage-filter-menu">
              <button type="button" onClick={() => { onEventKindChange(null); setKindOpen(false); }}>
                {t("usage.allKinds")}
              </button>
              {["api_call", "tool_call", "turn_summary"].map((k) => (
                <button
                  key={k}
                  type="button"
                  onClick={() => {
                    onEventKindChange(k);
                    setKindOpen(false);
                  }}
                >
                  {k}
                </button>
              ))}
            </div>
          )}
        </div>
        <div className="usage-filter-wrap">
          <button
            type="button"
            className={`usage-fchip ${model ? "on" : ""}`}
            onClick={() => {
              setModelOpen((v) => !v);
              setKindOpen(false);
            }}
          >
            {modelLabel}
            <Chevron />
          </button>
          {modelOpen && (
            <div className="usage-filter-menu">
              <button type="button" onClick={() => { onModelChange(null); setModelOpen(false); }}>
                {t("usage.allModels")}
              </button>
              {models.map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => {
                    onModelChange(m);
                    setModelOpen(false);
                  }}
                >
                  {m}
                </button>
              ))}
            </div>
          )}
        </div>
      </div>

      {events.length === 0 && !loading ? (
        <div className="usage-empty">
          <div className="et">{t("usage.emptyTitle")}</div>
        </div>
      ) : (
        <div className="usage-table">
          <div className="usage-table-head">
            <span>{t("usage.colTime")}</span>
            <span>{t("usage.colKind")}</span>
            <span>{t("usage.colWho")}</span>
            <span className="h-hide">{t("usage.colDetail")}</span>
            <span className="h-hide" style={{ textAlign: "right" }}>
              {t("usage.colIn")}
            </span>
            <span className="h-hide" style={{ textAlign: "right" }}>
              {t("usage.colOut")}
            </span>
            <span style={{ textAlign: "right" }}>{t("usage.colTotal")}</span>
            <span style={{ textAlign: "right" }}>{t("usage.colStatus")}</span>
          </div>
          {events.map((ev) => (
            <EventRow key={ev.id} ev={ev} />
          ))}
        </div>
      )}

      <div className="usage-card-foot">
        <span>{t("usage.totalRows", { count: approxTotal })}</span>
        <div className="usage-pager">
          <button
            type="button"
            disabled={page <= 0}
            onClick={() => onPageChange(page - 1)}
          >
            {t("usage.prevPage")}
          </button>
          <span className="pg">
            {t("usage.pageOf", {
              page: page + 1,
              pages: hasMore ? `${page + 1}+` : page + 1,
            })}
          </span>
          <button
            type="button"
            disabled={!hasMore}
            onClick={() => onPageChange(page + 1)}
          >
            {t("usage.nextPage")}
          </button>
        </div>
      </div>
    </div>
  );
}

function EventRow({ ev }: { ev: TokenUsageEventRow }) {
  const kind = ev.event_kind;
  const badgeClass =
    kind === "api_call" ? "api" : kind === "tool_call" ? "tool" : "turn";
  const badgeLabel =
    kind === "api_call" ? "api" : kind === "tool_call" ? "tool" : "turn";
  const who =
    kind === "tool_call"
      ? ev.tool_name || "—"
      : ev.model || "—";
  const detail = detailOf(ev);
  const hasTokens = kind === "api_call" || kind === "turn_summary";

  return (
    <div className="usage-table-row">
      <span className="time">{formatTimeOfDay(ev.created_at)}</span>
      <span className="kind">
        <span className={`usage-kind-badge ${badgeClass}`}>{badgeLabel}</span>
      </span>
      <span className={`who ${kind === "tool_call" ? "mono" : ""}`}>
        {who}
        {ev.provider && kind !== "tool_call" ? (
          <span className="prov"> {ev.provider}</span>
        ) : null}
      </span>
      <span className="detail h-hide">{detail}</span>
      <span className={`num h-hide ${hasTokens ? "" : "dim"}`}>
        {hasTokens && ev.prompt_tokens != null ? formatInt(ev.prompt_tokens) : "—"}
      </span>
      <span className={`num h-hide ${hasTokens ? "" : "dim"}`}>
        {hasTokens && ev.completion_tokens != null
          ? formatInt(ev.completion_tokens)
          : "—"}
      </span>
      <span className={`num ${hasTokens ? "" : "dim"}`}>
        {hasTokens && ev.total_tokens != null ? formatInt(ev.total_tokens) : "—"}
      </span>
      <span className="st">
        <span className={ev.is_error ? "dot-err" : "dot-ok"} />
      </span>
    </div>
  );
}

function detailOf(ev: TokenUsageEventRow): string {
  if (ev.event_kind === "turn_summary") {
    return ev.message_id ? `${ev.message_id.slice(0, 10)}…` : "turn";
  }
  if (ev.event_kind === "api_call") {
    const base = ev.turn_index != null ? `turn ${ev.turn_index}` : "api";
    const cacheRead = ev.cache_read_tokens ?? 0;
    const cacheWrite = ev.cache_write_tokens ?? 0;
    if (cacheRead > 0 || cacheWrite > 0) {
      const parts = [
        cacheRead > 0 ? `cache↓${formatInt(cacheRead)}` : null,
        cacheWrite > 0 ? `cache↑${formatInt(cacheWrite)}` : null,
      ].filter(Boolean);
      return `${base} · ${parts.join(" ")}`;
    }
    return base;
  }
  if (ev.metadata_json) {
    try {
      const m = JSON.parse(ev.metadata_json) as Record<string, unknown>;
      if (typeof m.path === "string") {
        const base = m.path.split(/[/\\]/).pop() ?? m.path;
        if (m.paragraph_number != null) return `${base} · P${m.paragraph_number}`;
        return base;
      }
    } catch {
      // ignore
    }
  }
  return ev.tool_name ?? "—";
}

function Chevron() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="6 9 12 15 18 9" />
    </svg>
  );
}

export { PAGE_SIZE };
