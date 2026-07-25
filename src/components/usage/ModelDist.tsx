import { useTranslation } from "react-i18next";
import type { ModelUsageRow } from "../../types";
import {
  cacheHitRate,
  formatCacheHitPct,
  formatCompactTokens,
} from "./format";

interface ModelDistProps {
  rows: ModelUsageRow[];
}

export function ModelDist({ rows }: ModelDistProps) {
  const { t } = useTranslation();
  const max = Math.max(1, ...rows.map((r) => r.total_tokens));
  const total = rows.reduce((s, r) => s + r.total_tokens, 0) || 1;

  return (
    <div className="usage-card">
      <div className="usage-card-head">
        <span className="t">{t("usage.byModel")}</span>
        <span className="m">{t("usage.byModelMeta")}</span>
      </div>
      {rows.length === 0 ? (
        <div className="usage-empty">
          <div className="et">{t("usage.emptyTitle")}</div>
        </div>
      ) : (
        <div className="usage-dist">
          {rows.map((row) => {
            const pct = (row.total_tokens / total) * 100;
            const width = (row.total_tokens / max) * 100;
            const hitPct = cacheHitRate(
              row.prompt_tokens,
              row.cache_read_tokens,
              row.provider,
            );
            return (
              <div className="usage-dist-row" key={`${row.model}|${row.provider ?? ""}`}>
                <span className="usage-dist-name">
                  {row.model || "—"}
                  {row.provider ? <span className="prov">{row.provider}</span> : null}
                </span>
                <div className="usage-dist-track">
                  <i style={{ width: `${width}%` }} />
                </div>
                <span className="usage-dist-split">
                  <span>
                    {t("usage.legendIn")}{" "}
                    <b>{formatCompactTokens(row.prompt_tokens)}</b>
                  </span>
                  <span>
                    {t("usage.legendOut")}{" "}
                    <b>{formatCompactTokens(row.completion_tokens)}</b>
                  </span>
                  {(row.cache_read_tokens > 0 || row.cache_write_tokens > 0) && (
                    <span title={t("usage.cacheHint")}>
                      {t("usage.legendCache")}{" "}
                      <b>
                        {formatCompactTokens(row.cache_read_tokens)}
                        {row.cache_write_tokens > 0
                          ? `/${formatCompactTokens(row.cache_write_tokens)}`
                          : ""}
                        {hitPct != null ? ` · ${formatCacheHitPct(hitPct)}` : ""}
                      </b>
                    </span>
                  )}
                </span>
                <span className="usage-dist-num">
                  {formatCompactTokens(row.total_tokens)}
                </span>
                <span className="usage-dist-pct">{pct.toFixed(1)}%</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
