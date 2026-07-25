import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DailyUsageRow } from "../../types";
import { formatCompactTokens, formatMdShort, todayDateStr } from "./format";

interface TrendChartProps {
  rows: DailyUsageRow[];
  fromLabel: string;
  toLabel: string;
}

/** Keep the chart light when the range has many days. */
function downsample(rows: DailyUsageRow[], maxPoints = 60): DailyUsageRow[] {
  if (rows.length <= maxPoints) return rows;
  const out: DailyUsageRow[] = [];
  const step = rows.length / maxPoints;
  for (let i = 0; i < maxPoints; i++) {
    const start = Math.floor(i * step);
    const end = Math.floor((i + 1) * step);
    const slice = rows.slice(start, Math.max(end, start + 1));
    out.push({
      date: slice[slice.length - 1]?.date ?? slice[0].date,
      prompt_tokens: slice.reduce((s, r) => s + r.prompt_tokens, 0),
      completion_tokens: slice.reduce((s, r) => s + r.completion_tokens, 0),
      total_tokens: slice.reduce((s, r) => s + r.total_tokens, 0),
      cache_read_tokens: slice.reduce((s, r) => s + (r.cache_read_tokens ?? 0), 0),
      cache_write_tokens: slice.reduce(
        (s, r) => s + (r.cache_write_tokens ?? 0),
        0,
      ),
      api_call_count: slice.reduce((s, r) => s + r.api_call_count, 0),
    });
  }
  return out;
}

export function TrendChart({ rows, fromLabel, toLabel }: TrendChartProps) {
  const { t } = useTranslation();
  const [hover, setHover] = useState<number | null>(null);
  const today = todayDateStr();
  const plotRows = useMemo(() => downsample(rows), [rows]);

  useEffect(() => {
    setHover(null);
  }, [plotRows]);

  const chart = useMemo(() => {
    const width = 800;
    const height = 190;
    const baseline = 164;
    const topPad = 20;
    const usable = baseline - topPad;
    const n = Math.max(plotRows.length, 1);
    const slot = width / n;
    const barW = Math.min(44, Math.max(18, slot * 0.55));
    const max = Math.max(
      1,
      ...plotRows.map((r) => r.prompt_tokens + r.completion_tokens),
    );
    const bars = plotRows.map((r, i) => {
      const inH = (r.prompt_tokens / max) * usable;
      const outH = (r.completion_tokens / max) * usable;
      const x = slot * i + (slot - barW) / 2;
      const inY = baseline - inH;
      const outY = inY - outH;
      return { ...r, x, barW, inY, inH, outY, outH, cx: x + barW / 2 };
    });
    return { width, height, baseline, bars };
  }, [plotRows]);

  const tipIdx = hover ?? (plotRows.length > 0 ? plotRows.length - 1 : null);
  const tip = tipIdx != null ? chart.bars[tipIdx] : null;

  if (plotRows.length === 0) {
    return (
      <div className="usage-card">
        <div className="usage-card-head">
          <span className="t">{t("usage.trendTitle")}</span>
          <span className="m">{t("usage.trendMeta", { from: fromLabel, to: toLabel })}</span>
        </div>
        <div className="usage-empty">
          <div className="et">{t("usage.emptyTitle")}</div>
          <div className="ed">{t("usage.emptyDesc")}</div>
        </div>
      </div>
    );
  }

  const tipX = tip
    ? Math.min(Math.max(tip.cx - 66, 8), chart.width - 140)
    : 0;

  return (
    <div className="usage-card">
      <div className="usage-card-head">
        <span className="t">{t("usage.trendTitle")}</span>
        <span className="m">{t("usage.trendMeta", { from: fromLabel, to: toLabel })}</span>
        <div className="usage-legend">
          <span>
            <i style={{ background: "var(--ink)" }} />
            {t("usage.legendIn")}
          </span>
          <span>
            <i
              style={{
                background: "color-mix(in srgb, var(--ink) 45%, transparent)",
              }}
            />
            {t("usage.legendOut")}
          </span>
        </div>
      </div>
      <div className="usage-chart-wrap">
        <svg
          viewBox={`0 0 ${chart.width} ${chart.height}`}
          role="img"
          aria-label={t("usage.trendTitle")}
          onMouseLeave={() => setHover(null)}
        >
          <line
            x1="0"
            y1={chart.baseline}
            x2={chart.width}
            y2={chart.baseline}
            stroke="var(--line-strong)"
            strokeWidth="1"
          />
          {chart.bars.map((b, i) => (
            <g
              key={b.date}
              onMouseEnter={() => setHover(i)}
              style={{ cursor: "default" }}
            >
              <rect
                x={b.x}
                y={0}
                width={b.barW}
                height={chart.baseline}
                fill="transparent"
              />
              {b.inH > 0 && (
                <rect
                  x={b.x}
                  y={b.inY}
                  width={b.barW}
                  height={b.inH}
                  rx="3"
                  fill="var(--ink)"
                />
              )}
              {b.outH > 0 && (
                <rect
                  x={b.x}
                  y={b.outY}
                  width={b.barW}
                  height={b.outH}
                  rx="3"
                  fill="var(--ink)"
                  opacity="0.45"
                />
              )}
            </g>
          ))}
          {tip && (
            <>
              <line
                x1={tip.cx}
                y1="8"
                x2={tip.cx}
                y2={chart.baseline}
                stroke="var(--line-strong)"
                strokeWidth="1"
                strokeDasharray="3 3"
              />
              <g>
                <rect
                  x={tipX}
                  y="10"
                  width="132"
                  height="48"
                  rx="8"
                  fill="var(--bg)"
                  stroke="var(--line-strong)"
                />
                <text
                  x={tipX + 12}
                  y="28"
                  fontSize="11"
                  fontWeight="600"
                  fill="var(--ink)"
                  fontFamily="Instrument Sans, sans-serif"
                >
                  {formatMdShort(tip.date)}
                  {tip.date === today ? ` · ${t("usage.today")}` : ""}
                </text>
                <text
                  x={tipX + 12}
                  y="46"
                  fontSize="10.5"
                  fill="var(--ink-mute)"
                  fontFamily="JetBrains Mono, monospace"
                >
                  in {formatCompactTokens(tip.prompt_tokens)} · out{" "}
                  {formatCompactTokens(tip.completion_tokens)}
                </text>
              </g>
            </>
          )}
        </svg>
      </div>
      {/* Equal columns match SVG bar slots so labels sit under each bar center. */}
      <div
        className="usage-chart-x"
        style={{ gridTemplateColumns: `repeat(${plotRows.length}, minmax(0, 1fr))` }}
      >
        {plotRows.map((r, i) => {
          const step = Math.max(1, Math.ceil(plotRows.length / 8));
          const show =
            plotRows.length <= 8 ||
            i === 0 ||
            i === plotRows.length - 1 ||
            i % step === 0;
          return (
            <span key={`${r.date}-${i}`} className={show ? undefined : "is-hidden"}>
              {formatMdShort(r.date)}
            </span>
          );
        })}
      </div>
    </div>
  );
}
