import { useTranslation } from "react-i18next";
import type { TokenUsageSummary } from "../../types";
import {
  formatCompactTokens,
  formatInt,
  formatMoney,
  formatPctChange,
} from "./format";

interface StatStripProps {
  summary: TokenUsageSummary | null;
  prevSummary: TokenUsageSummary | null;
  toolErrorCount: number;
  cost: { total: number; input: number; output: number } | null;
  costEnabled: boolean;
  currency: string;
}

export function StatStrip({
  summary,
  prevSummary,
  toolErrorCount,
  cost,
  costEnabled,
  currency,
}: StatStripProps) {
  const { t } = useTranslation();
  const total = summary?.total_tokens ?? 0;
  const prompt = summary?.prompt_tokens ?? 0;
  const cacheRead = summary?.cache_read_tokens ?? 0;
  const cacheWrite = summary?.cache_write_tokens ?? 0;
  const api = summary?.api_call_count ?? 0;
  const tools = summary?.tool_call_count ?? 0;
  const pct = prevSummary
    ? formatPctChange(total, prevSummary.total_tokens)
    : null;
  const avg = api > 0 ? formatCompactTokens(total / api) : "0";
  const failRate =
    tools > 0 && toolErrorCount > 0
      ? ((toolErrorCount / tools) * 100).toFixed(2)
      : null;
  // OpenAI folds cached into prompt; Claude keeps them separate.
  const cacheDenom = Math.max(prompt, prompt + cacheRead);
  const cacheHitPct =
    cacheRead > 0 && cacheDenom > 0
      ? Math.min(100, (cacheRead / cacheDenom) * 100)
      : null;

  return (
    <div className="usage-stats">
      <div className="usage-stat">
        <div className="usage-stat-label">
          <BoltIcon />
          {t("usage.totalTokens")}
        </div>
        <div className="usage-stat-value">{formatCompactTokens(total)}</div>
        <div className="usage-stat-sub">
          {cacheRead + cacheWrite > 0 ? (
            <span className="mono" title={t("usage.cacheHint")}>
              {t("usage.cacheSplit", {
                read: formatCompactTokens(cacheRead),
                write: formatCompactTokens(cacheWrite),
                hit: cacheHitPct != null ? `${cacheHitPct.toFixed(0)}%` : "—",
              })}
            </span>
          ) : (
            <>
              {pct != null ? (
                <span className={pct >= 0 ? "up" : "down"}>
                  {pct >= 0 ? "▲" : "▼"} {Math.abs(pct).toFixed(1)}%
                </span>
              ) : null}
              <span>{t("usage.vsPrev")}</span>
            </>
          )}
        </div>
      </div>
      <div className="usage-stat">
        <div className="usage-stat-label">
          <ChatIcon />
          {t("usage.apiCalls")}
        </div>
        <div className="usage-stat-value">{formatInt(api)}</div>
        <div className="usage-stat-sub">
          <span className="mono">{t("usage.avgTokPerCall", { avg })}</span>
        </div>
      </div>
      <div className="usage-stat">
        <div className="usage-stat-label">
          <WrenchIcon />
          {t("usage.toolCalls")}
        </div>
        <div className="usage-stat-value">{formatInt(tools)}</div>
        <div className="usage-stat-sub">
          {toolErrorCount > 0 ? (
            <>
              <span className="mono">
                {t("usage.toolFailRate", { count: toolErrorCount })}
              </span>
              {failRate != null ? <span className="down">{failRate}%</span> : null}
            </>
          ) : (
            <span className="mono">—</span>
          )}
        </div>
      </div>
      <div className="usage-stat">
        <div className="usage-stat-label">
          <CostIcon />
          {t("usage.estCost")}
        </div>
        <div className="usage-stat-value">
          {costEnabled && cost ? (
            <>
              <span className="unit">{currency}</span>
              {cost.total.toFixed(2)}
            </>
          ) : (
            "—"
          )}
        </div>
        <div className="usage-stat-sub">
          {costEnabled && cost ? (
            <span className="mono">
              {t("usage.costSplit", {
                input: formatMoney(cost.input, ""),
                output: formatMoney(cost.output, ""),
              })}
            </span>
          ) : (
            <span className="mono">—</span>
          )}
        </div>
      </div>
    </div>
  );
}

function BoltIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
    </svg>
  );
}
function ChatIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
    </svg>
  );
}
function WrenchIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z" />
    </svg>
  );
}
function CostIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
      <line x1="12" y1="1" x2="12" y2="23" />
      <path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6" />
    </svg>
  );
}
