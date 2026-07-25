export type UsageScope = "today" | "7d" | "30d" | "all";

export interface TimeRange {
  fromMs: number | null;
  toMs: number | null;
}

function startOfLocalDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate(), 0, 0, 0, 0);
}

function endOfLocalDay(d: Date): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate(), 23, 59, 59, 999);
}

/** Inclusive window for the selected scope. */
export function rangeForScope(scope: UsageScope, now = new Date()): TimeRange {
  const toMs = now.getTime();
  if (scope === "all") {
    return { fromMs: null, toMs: null };
  }
  if (scope === "today") {
    return { fromMs: startOfLocalDay(now).getTime(), toMs };
  }
  const days = scope === "7d" ? 7 : 30;
  const from = startOfLocalDay(now);
  from.setDate(from.getDate() - (days - 1));
  return { fromMs: from.getTime(), toMs };
}

/** Previous window of equal length, ending just before the current window. */
export function previousRange(scope: UsageScope, now = new Date()): TimeRange | null {
  if (scope === "all") return null;
  const current = rangeForScope(scope, now);
  if (current.fromMs == null) return null;
  const duration = (current.toMs ?? now.getTime()) - current.fromMs;
  const prevTo = current.fromMs - 1;
  const prevFrom = prevTo - duration + 1;
  return { fromMs: prevFrom, toMs: prevTo };
}

export function formatCompactTokens(n: number): string {
  const abs = Math.abs(n);
  if (abs >= 1_000_000) {
    const v = n / 1_000_000;
    return `${v >= 10 || v <= -10 ? v.toFixed(1) : v.toFixed(2).replace(/\.?0+$/, "")}M`;
  }
  if (abs >= 1_000) {
    const v = n / 1_000;
    return `${v >= 100 || v <= -100 ? Math.round(v) : v.toFixed(1).replace(/\.0$/, "")}K`;
  }
  return new Intl.NumberFormat().format(Math.round(n));
}

export function formatInt(n: number): string {
  return new Intl.NumberFormat().format(Math.round(n));
}

export function formatMoney(n: number, currency = "¥"): string {
  const fixed = n < 10 ? n.toFixed(2) : n.toFixed(2);
  return `${currency}${fixed}`;
}

export function formatPctChange(current: number, previous: number): number | null {
  if (previous <= 0) return current > 0 ? 100 : null;
  return ((current - previous) / previous) * 100;
}

/** Claude/Anthropic report cache tokens outside `input_tokens`; OpenAI folds them in. */
export function isSeparateCacheProvider(provider?: string | null): boolean {
  const p = (provider ?? "").toLowerCase();
  return p === "claude" || p.includes("anthropic");
}

/**
 * Denominator for cache hit rate.
 * - Claude: prompt + cache_read
 * - OpenAI-style (known provider): prompt (cache ⊆ prompt)
 * - Unknown: if cache_read ≤ prompt treat as folded, else separate
 */
export function cacheHitDenom(
  promptTokens: number,
  cacheReadTokens: number,
  provider?: string | null,
): number {
  if (cacheReadTokens <= 0) return 0;
  if (promptTokens <= 0) return cacheReadTokens;
  if (isSeparateCacheProvider(provider)) {
    return promptTokens + cacheReadTokens;
  }
  if (provider) {
    return promptTokens;
  }
  return cacheReadTokens <= promptTokens
    ? promptTokens
    : promptTokens + cacheReadTokens;
}

/** Cache hit rate in 0–100, or null when there is no cache read. */
export function cacheHitRate(
  promptTokens: number,
  cacheReadTokens: number,
  provider?: string | null,
): number | null {
  if (cacheReadTokens <= 0) return null;
  const denom = cacheHitDenom(promptTokens, cacheReadTokens, provider);
  if (denom <= 0) return null;
  return Math.min(100, (cacheReadTokens / denom) * 100);
}

/** Weighted hit rate across model rows (preferred for period summaries). */
export function aggregateCacheHitRate(
  rows: Array<{
    prompt_tokens: number;
    cache_read_tokens: number;
    provider?: string | null;
  }>,
): number | null {
  let read = 0;
  let denom = 0;
  for (const row of rows) {
    const r = row.cache_read_tokens ?? 0;
    if (r <= 0) continue;
    const d = cacheHitDenom(row.prompt_tokens ?? 0, r, row.provider);
    if (d > 0) {
      read += r;
      denom += d;
    }
  }
  if (read <= 0 || denom <= 0) return null;
  return Math.min(100, (read / denom) * 100);
}

export function formatCacheHitPct(rate: number | null): string {
  if (rate == null) return "—";
  return `${rate.toFixed(rate >= 10 ? 0 : 1)}%`;
}

export function formatMdShort(dateStr: string): string {
  // YYYY-MM-DD → MM-DD
  if (/^\d{4}-\d{2}-\d{2}$/.test(dateStr)) {
    return dateStr.slice(5);
  }
  return dateStr;
}

export function formatTimeOfDay(ms: number): string {
  const d = new Date(ms);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  const ss = String(d.getSeconds()).padStart(2, "0");
  return `${hh}:${mm}:${ss}`;
}

export function todayDateStr(now = new Date()): string {
  const y = now.getFullYear();
  const m = String(now.getMonth() + 1).padStart(2, "0");
  const d = String(now.getDate()).padStart(2, "0");
  return `${y}-${m}-${d}`;
}

export function localDateStr(ms: number): string {
  return todayDateStr(new Date(ms));
}

/** Max days to zero-fill; beyond this, keep sparse rows from the API. */
const MAX_GAP_FILL_DAYS = 92;

export function fillDailyGaps<
  T extends {
    date: string;
    prompt_tokens: number;
    completion_tokens: number;
    total_tokens: number;
    cache_read_tokens?: number;
    cache_write_tokens?: number;
    api_call_count?: number;
  },
>(rows: T[], fromMs: number | null, toMs: number | null): T[] {
  if (fromMs == null || toMs == null) return rows;
  const dayMs = 86_400_000;
  const spanDays =
    (startOfLocalDay(new Date(toMs)).getTime() -
      startOfLocalDay(new Date(fromMs)).getTime()) /
      dayMs +
    1;
  if (spanDays > MAX_GAP_FILL_DAYS) return rows;

  const map = new Map(rows.map((r) => [r.date, r]));
  const out: T[] = [];
  const cursor = startOfLocalDay(new Date(fromMs));
  const end = startOfLocalDay(new Date(toMs));
  while (cursor.getTime() <= end.getTime()) {
    const key = todayDateStr(cursor);
    const existing = map.get(key);
    out.push(
      existing ??
        ({
          date: key,
          prompt_tokens: 0,
          completion_tokens: 0,
          total_tokens: 0,
          cache_read_tokens: 0,
          cache_write_tokens: 0,
          api_call_count: 0,
        } as T),
    );
    cursor.setDate(cursor.getDate() + 1);
  }
  return out;
}

export { endOfLocalDay };
