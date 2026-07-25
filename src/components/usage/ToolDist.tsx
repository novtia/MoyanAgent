import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { ToolUsageRow } from "../../types";
import { formatInt } from "./format";

const TOP_N = 5;

interface DisplayRow {
  key: string;
  name: string;
  call_count: number;
}

interface ToolDistProps {
  rows: ToolUsageRow[];
}

export function ToolDist({ rows }: ToolDistProps) {
  const { t } = useTranslation();
  const errorTotal = rows.reduce((s, r) => s + r.error_count, 0);

  const display = useMemo((): DisplayRow[] => {
    if (rows.length <= TOP_N + 1) {
      return rows.map((r) => ({
        key: r.tool_name || "__empty",
        name: r.tool_name || "—",
        call_count: r.call_count,
      }));
    }
    const head = rows.slice(0, TOP_N);
    const rest = rows.slice(TOP_N);
    return [
      ...head.map((r) => ({
        key: r.tool_name || "__empty",
        name: r.tool_name || "—",
        call_count: r.call_count,
      })),
      {
        key: "__other",
        name: t("usage.otherTools", { count: rest.length }),
        call_count: rest.reduce((s, r) => s + r.call_count, 0),
      },
    ];
  }, [rows, t]);

  const max = Math.max(1, ...display.map((r) => r.call_count));
  const total = rows.reduce((s, r) => s + r.call_count, 0) || 1;

  return (
    <div className="usage-card">
      <div className="usage-card-head">
        <span className="t">{t("usage.byTool")}</span>
        <span className="m">{t("usage.byToolMeta", { count: errorTotal })}</span>
      </div>
      {rows.length === 0 ? (
        <div className="usage-empty">
          <div className="et">{t("usage.emptyTitle")}</div>
        </div>
      ) : (
        <div className="usage-dist">
          {display.map((row) => {
            const pct = (row.call_count / total) * 100;
            const width = (row.call_count / max) * 100;
            return (
              <div className="usage-dist-row muted" key={row.key}>
                <span className="usage-dist-name mono">{row.name}</span>
                <div className="usage-dist-track">
                  <i style={{ width: `${width}%` }} />
                </div>
                <span className="usage-dist-num">{formatInt(row.call_count)}</span>
                <span className="usage-dist-pct">{pct.toFixed(1)}%</span>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
