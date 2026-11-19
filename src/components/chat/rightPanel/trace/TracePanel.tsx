import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../../api/tauri";
import { useSession } from "../../../../store/session";
import type { SessionTraceEntry } from "../../../../types";
import {
  buildTraceModel,
  filterTraceRounds,
  formatDurationMs,
} from "./traceModel";
import { TraceTimeline } from "./TraceTimeline";
import { TraceRow } from "./TraceRow";

export interface TracePanelProps {
  open: boolean;
}

export function TracePanel({ open }: TracePanelProps) {
  const { t } = useTranslation();
  const sessionId = useSession((s) => s.active?.session.id ?? null);
  const busy = useSession((s) => s.busy);

  const [entries, setEntries] = useState<SessionTraceEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const [showDuration, setShowDuration] = useState(true);
  const [groupByRound, setGroupByRound] = useState(true);
  const [showCalls, setShowCalls] = useState(true);
  const [query, setQuery] = useState("");
  const [activeRowId, setActiveRowId] = useState<string | null>(null);
  const [collapsedRounds, setCollapsedRounds] = useState<Record<number, boolean>>({});

  const listRef = useRef<HTMLDivElement | null>(null);
  const prevBusyRef = useRef(busy);

  const fetchTrace = useCallback(async () => {
    if (!sessionId) {
      setEntries([]);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const list = await api.getSessionTrace(sessionId);
      setEntries(Array.isArray(list) ? list : []);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionId]);

  // Fetch when panel becomes visible or session switches.
  useEffect(() => {
    if (!open) return;
    void fetchTrace();
  }, [open, sessionId, fetchTrace]);

  // Auto-refresh when a generation finishes (busy: true -> false).
  useEffect(() => {
    const wasBusy = prevBusyRef.current;
    prevBusyRef.current = busy;
    if (open && wasBusy && !busy) {
      void fetchTrace();
    }
  }, [open, busy, fetchTrace]);

  const model = useMemo(() => buildTraceModel(entries), [entries]);
  const visibleRounds = useMemo(
    () => filterTraceRounds(model.rounds, query, showCalls),
    [model.rounds, query, showCalls],
  );

  const handleSelectRow = useCallback((rowId: string) => {
    setActiveRowId(rowId);
    const el = document.getElementById(`trace-row-${rowId}`);
    if (el) {
      el.scrollIntoView({ behavior: "smooth", block: "center" });
    }
  }, []);

  const toggleRound = useCallback((index: number) => {
    setCollapsedRounds((prev) => ({ ...prev, [index]: !prev[index] }));
  }, []);

  if (!sessionId) {
    return (
      <div className="trace-panel">
        <div className="trace-empty">{t("trace.noSession")}</div>
      </div>
    );
  }

  return (
    <div className="trace-panel">
      <div className="trace-toolbar">
        <div className="trace-toggles">
          <button
            type="button"
            className={`trace-toggle-btn${showDuration ? " is-active" : ""}`}
            aria-pressed={showDuration}
            onClick={() => setShowDuration((v) => !v)}
          >
            {t("trace.toggle.duration")}
          </button>
          <button
            type="button"
            className={`trace-toggle-btn${groupByRound ? " is-active" : ""}`}
            aria-pressed={groupByRound}
            onClick={() => setGroupByRound((v) => !v)}
          >
            {t("trace.toggle.rounds")}
          </button>
          <button
            type="button"
            className={`trace-toggle-btn${showCalls ? " is-active" : ""}`}
            aria-pressed={showCalls}
            onClick={() => setShowCalls((v) => !v)}
          >
            {t("trace.toggle.calls")}
          </button>
        </div>

        <div className="trace-toolbar-right">
          <input
            type="search"
            className="trace-search"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t("trace.searchPlaceholder")}
            aria-label={t("trace.searchPlaceholder")}
          />
          <button
            type="button"
            className="ghost-btn trace-refresh-btn"
            title={t("trace.refresh")}
            aria-label={t("trace.refresh")}
            disabled={loading}
            onClick={() => void fetchTrace()}
          >
            <RefreshIcon />
          </button>
        </div>
      </div>

      {entries.length > 0 && (
        <div className="trace-summary-bar">
          <span>{t("trace.summary.rounds", { count: model.rounds.length })}</span>
          <span>{t("trace.summary.calls", { api: model.totalApiCalls, tool: model.totalToolCalls })}</span>
          {model.totalTokens > 0 && (
            <span>{model.totalTokens.toLocaleString()} tok</span>
          )}
          {model.hasTiming && (
            <span>{formatDurationMs(model.totalDurationMs)}</span>
          )}
        </div>
      )}

      <TraceTimeline
        segments={model.segments}
        byDuration={showDuration && model.hasTiming}
        activeRowId={activeRowId}
        onSelectRow={handleSelectRow}
      />

      <div className="trace-list" ref={listRef}>
        {loading && entries.length === 0 ? (
          <div className="trace-empty">{t("trace.loading")}</div>
        ) : error ? (
          <div className="trace-empty is-error">{error}</div>
        ) : visibleRounds.length === 0 ? (
          <div className="trace-empty">{t("trace.empty")}</div>
        ) : groupByRound ? (
          visibleRounds.map((round) => {
            const collapsed = !!collapsedRounds[round.index];
            const roundDur = showDuration && round.hasTiming
              ? formatDurationMs(round.totalDurationMs)
              : "";
            return (
              <div key={round.index} className="trace-round">
                <button
                  type="button"
                  className="trace-round-head"
                  onClick={() => toggleRound(round.index)}
                  aria-expanded={!collapsed}
                >
                  <span className="trace-round-caret">{collapsed ? "▸" : "▾"}</span>
                  <span className="trace-round-title">
                    {t("trace.roundLabel", { index: round.index })}
                  </span>
                  <span className="trace-round-meta">
                    {round.totalTokens > 0 && `${round.totalTokens.toLocaleString()} tok`}
                    {round.totalTokens > 0 && roundDur ? " · " : ""}
                    {roundDur}
                  </span>
                </button>
                {!collapsed && (
                  <div className="trace-round-rows">
                    {round.rows.map((row) => (
                      <TraceRow
                        key={row.id}
                        row={row}
                        showDuration={showDuration}
                        highlighted={activeRowId === row.id}
                      />
                    ))}
                  </div>
                )}
              </div>
            );
          })
        ) : (
          <div className="trace-round-rows">
            {visibleRounds.flatMap((round) =>
              round.rows.map((row) => (
                <TraceRow
                  key={row.id}
                  row={row}
                  showDuration={showDuration}
                  highlighted={activeRowId === row.id}
                />
              )),
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function RefreshIcon() {
  return (
    <svg
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinecap="round"
      strokeLinejoin="round"
    >
      <path d="M21 12a9 9 0 1 1-2.64-6.36" />
      <polyline points="21 3 21 9 15 9" />
    </svg>
  );
}
