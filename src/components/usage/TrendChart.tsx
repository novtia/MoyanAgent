import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { DailyUsageRow } from "../../types";
import { formatCompactTokens, formatMdShort, todayDateStr } from "./format";

interface TrendChartProps {
  rows: DailyUsageRow[];
  fromLabel: string;
  toLabel: string;
}

/** Axis label: 07-01 → 7-1 */
function formatMdAxis(dateStr: string): string {
  return formatMdShort(dateStr).replace(/^0/, "").replace(/-0/, "-");
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

type BarGeom = DailyUsageRow & {
  x: number;
  barW: number;
  cx: number;
  inFreshY: number;
  inFreshH: number;
  cacheY: number;
  cacheH: number;
  outY: number;
  outH: number;
};

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
    const labelY = baseline + 16;
    const topPad = 20;
    const usable = baseline - topPad;
    const n = Math.max(plotRows.length, 1);
    const slot = width / n;
    const barW = Math.min(44, Math.max(18, slot * 0.55));

    // Split prompt into fresh + cached so OpenAI-style (cache ⊆ prompt) does not
    // double-count; leftover cache beyond prompt still stacks on top.
    const parts = plotRows.map((r) => {
      const cacheRead = r.cache_read_tokens ?? 0;
      const inCached = Math.min(cacheRead, r.prompt_tokens);
      const inFresh = Math.max(0, r.prompt_tokens - inCached);
      const cacheExtra = Math.max(0, cacheRead - r.prompt_tokens);
      const cacheH = inCached + cacheExtra;
      const stack = inFresh + cacheH + r.completion_tokens;
      return { inFresh, cacheH, out: r.completion_tokens, stack };
    });
    const max = Math.max(1, ...parts.map((p) => p.stack));

    const bars: BarGeom[] = plotRows.map((r, i) => {
      const p = parts[i];
      const inFreshH = (p.inFresh / max) * usable;
      const cacheH = (p.cacheH / max) * usable;
      const outH = (p.out / max) * usable;
      const x = slot * i + (slot - barW) / 2;
      const inFreshY = baseline - inFreshH;
      const cacheY = inFreshY - cacheH;
      const outY = cacheY - outH;
      return {
        ...r,
        x,
        barW,
        inFreshY,
        inFreshH,
        cacheY,
        cacheH,
        outY,
        outH,
        cx: x + barW / 2,
      };
    });
    return { width, height, baseline, labelY, bars };
  }, [plotRows]);

  // Only show tip while hovering — never pin to the last bar.
  const tip = hover != null ? chart.bars[hover] ?? null : null;

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

  const tipW = 168;
  const tipX = tip
    ? Math.min(Math.max(tip.cx - tipW / 2, 8), chart.width - tipW - 8)
    : 0;
  const tipCache = tip
    ? (tip.cache_read_tokens ?? 0) + (tip.cache_write_tokens ?? 0)
    : 0;
  const tipH = tipCache > 0 ? 62 : 48;

  const firstDate = plotRows[0]?.date;
  const lastDate = plotRows[plotRows.length - 1]?.date;
  const sameDay = firstDate === lastDate;

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
          <span>
            <i
              style={{
                background: "color-mix(in srgb, var(--blue-500) 70%, transparent)",
              }}
            />
            {t("usage.legendCache")}
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
              key={`${b.date}-${i}`}
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
              {b.inFreshH > 0 && (
                <rect
                  x={b.x}
                  y={b.inFreshY}
                  width={b.barW}
                  height={b.inFreshH}
                  rx="3"
                  fill="var(--ink)"
                />
              )}
              {b.cacheH > 0 && (
                <rect
                  x={b.x}
                  y={b.cacheY}
                  width={b.barW}
                  height={b.cacheH}
                  rx="3"
                  fill="color-mix(in srgb, var(--blue-500) 70%, transparent)"
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
                  width={tipW}
                  height={tipH}
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
                  {t("usage.tipInOut", {
                    in: formatCompactTokens(tip.prompt_tokens),
                    out: formatCompactTokens(tip.completion_tokens),
                  })}
                </text>
                {tipCache > 0 && (
                  <text
                    x={tipX + 12}
                    y="62"
                    fontSize="10.5"
                    fill="var(--ink-mute)"
                    fontFamily="JetBrains Mono, monospace"
                  >
                    {t("usage.tipCache", {
                      cache:
                        formatCompactTokens(tip.cache_read_tokens ?? 0) +
                        ((tip.cache_write_tokens ?? 0) > 0
                          ? `/${formatCompactTokens(tip.cache_write_tokens ?? 0)}`
                          : ""),
                    })}
                  </text>
                )}
              </g>
            </>
          )}
          {/* Axis dates sit on bar centers so they stay aligned with columns. */}
          {chart.bars[0] && firstDate && (
            <text
              x={chart.bars[0].cx}
              y={chart.labelY}
              textAnchor="middle"
              fontSize="10.5"
              fill="var(--ink-mute)"
              fontFamily="Instrument Sans, sans-serif"
              style={{ fontVariantNumeric: "tabular-nums" }}
            >
              {formatMdAxis(firstDate)}
            </text>
          )}
          {!sameDay && chart.bars.length > 1 && lastDate && (
            <text
              x={chart.bars[chart.bars.length - 1].cx}
              y={chart.labelY}
              textAnchor="middle"
              fontSize="10.5"
              fill="var(--ink-mute)"
              fontFamily="Instrument Sans, sans-serif"
              style={{ fontVariantNumeric: "tabular-nums" }}
            >
              {formatMdAxis(lastDate)}
            </text>
          )}
        </svg>
      </div>
    </div>
  );
}
