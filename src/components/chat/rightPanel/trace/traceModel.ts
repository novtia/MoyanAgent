import type { SessionTraceEntry } from "../../../../types";

export type TraceRowCategory =
  | "system"
  | "context"
  | "user"
  | "assistant"
  | "tool"
  | "error";

export interface TraceUsage {
  prompt_tokens?: number | null;
  completion_tokens?: number | null;
  total_tokens?: number | null;
  cache_read_tokens?: number | null;
  cache_write_tokens?: number | null;
}

export interface TraceRowItem {
  id: string;
  entry: SessionTraceEntry;
  category: TraceRowCategory;
  badgeLabelKey: string;
  summary: string;
  durationMs: number | null;
  startedAt: number;
  model?: string;
  provider?: string;
  toolName?: string;
  isError?: boolean;
  usage?: TraceUsage;
  turnIndex?: number | null;
  searchText: string;
}

export interface TraceRound {
  index: number;
  correlationId: string | null;
  rows: TraceRowItem[];
  startedAt: number;
  endedAt: number;
  totalDurationMs: number;
  hasTiming: boolean;
  totalTokens: number;
  apiCalls: number;
  toolCalls: number;
}

export type TimelineLane = "input" | "model" | "tool";

export interface TraceTimelineSegment {
  rowId: string;
  roundIndex: number;
  lane: TimelineLane;
  category: TraceRowCategory;
  isError?: boolean;
  label: string;
  durationMs: number | null;
  /** Normalized 0..100 offset and width in duration mode */
  leftPct: number;
  widthPct: number;
}

export interface TraceModel {
  rounds: TraceRound[];
  segments: TraceTimelineSegment[];
  totalDurationMs: number;
  hasTiming: boolean;
  totalTokens: number;
  totalApiCalls: number;
  totalToolCalls: number;
}

function asString(v: unknown): string {
  return typeof v === "string" ? v : "";
}

function formatPreview(text: string, max = 140): string {
  const oneLine = text.replace(/\s+/g, " ").trim();
  if (oneLine.length <= max) return oneLine;
  return `${oneLine.slice(0, max)}…`;
}

function safeJsonString(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "string") return v;
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}

function parseUsage(raw: unknown): TraceUsage | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const u = raw as Record<string, unknown>;
  const num = (k: string) => (typeof u[k] === "number" ? (u[k] as number) : null);
  const usage: TraceUsage = {
    prompt_tokens: num("prompt_tokens"),
    completion_tokens: num("completion_tokens"),
    total_tokens: num("total_tokens"),
    cache_read_tokens: num("cache_read_tokens"),
    cache_write_tokens: num("cache_write_tokens"),
  };
  if (
    usage.prompt_tokens == null &&
    usage.completion_tokens == null &&
    usage.total_tokens == null
  ) {
    return undefined;
  }
  return usage;
}

function classifyEntry(entry: SessionTraceEntry): TraceRowItem | null {
  const data = entry.data ?? {};

  if (entry.kind === "turn_summary") {
    return null;
  }

  if (entry.kind === "session_settings") {
    const sys = asString(data.system_prompt);
    const model = asString(data.model);
    const provider = asString(data.provider);
    const summary = sys
      ? formatPreview(sys)
      : model
        ? `${provider ? `${provider} / ` : ""}${model}`
        : "";
    return {
      id: entry.id,
      entry,
      category: "system",
      badgeLabelKey: "trace.badge.system",
      summary,
      durationMs: entry.duration_ms ?? null,
      startedAt: entry.started_at ?? entry.ts,
      model: model || undefined,
      provider: provider || undefined,
      searchText: `${sys} ${model} ${provider} ${asString(data.agent_type)}`.toLowerCase(),
    };
  }

  if (entry.kind === "context") {
    const label = asString(data.label) || asString(data.source) || "context";
    const content = asString(data.content);
    const summary = `${label}${content ? `: ${formatPreview(content, 110)}` : ""}`;
    return {
      id: entry.id,
      entry,
      category: "context",
      badgeLabelKey: "trace.badge.context",
      summary,
      durationMs: entry.duration_ms ?? null,
      startedAt: entry.started_at ?? entry.ts,
      searchText: `${label} ${asString(data.source)} ${content}`.toLowerCase(),
    };
  }

  if (entry.kind === "user_message") {
    const text = asString(data.text);
    return {
      id: entry.id,
      entry,
      category: "user",
      badgeLabelKey: "trace.badge.user",
      summary: formatPreview(text),
      durationMs: entry.duration_ms ?? null,
      startedAt: entry.started_at ?? entry.ts,
      searchText: text.toLowerCase(),
    };
  }

  if (entry.kind === "assistant_turn") {
    const model = asString(data.model);
    const provider = asString(data.provider);
    const text = asString(data.text);
    const thinking = asString(data.thinking);
    const toolCalls = Array.isArray(data.tool_calls)
      ? data.tool_calls.filter((x): x is string => typeof x === "string")
      : [];
    const usage = parseUsage(data.usage);

    let summary = "";
    if (text) {
      summary = formatPreview(text);
    } else if (toolCalls.length > 0) {
      summary = `→ ${toolCalls.join(", ")}`;
    } else if (thinking) {
      summary = formatPreview(thinking);
    } else if (model) {
      summary = model;
    }

    return {
      id: entry.id,
      entry,
      category: "assistant",
      badgeLabelKey: "trace.badge.assistant",
      summary,
      durationMs: entry.duration_ms ?? null,
      startedAt: entry.started_at ?? entry.ts,
      model: model || undefined,
      provider: provider || undefined,
      usage,
      turnIndex: entry.turn_index ?? null,
      searchText: `${model} ${provider} ${text} ${thinking} ${toolCalls.join(" ")}`.toLowerCase(),
    };
  }

  if (entry.kind === "tool_call") {
    const toolName = entry.tool_name || "Tool";
    const isError = Boolean(data.is_error);
    const inputStr = safeJsonString(data.input);
    const outputStr = safeJsonString(isError ? (data.error ?? data.output) : data.output);
    const summary = `${toolName}${inputStr ? `  ${formatPreview(inputStr, 96)}` : ""}`;
    return {
      id: entry.id,
      entry,
      category: "tool",
      badgeLabelKey: "trace.badge.tool",
      summary,
      durationMs: entry.duration_ms ?? null,
      startedAt: entry.started_at ?? entry.ts,
      toolName,
      isError,
      searchText: `${toolName} ${inputStr} ${outputStr}`.toLowerCase(),
    };
  }

  // Fallback / error
  const msg = asString(data.message) || safeJsonString(data);
  return {
    id: entry.id,
    entry,
    category: "error",
    badgeLabelKey: "trace.badge.error",
    summary: formatPreview(msg),
    durationMs: entry.duration_ms ?? null,
    startedAt: entry.started_at ?? entry.ts,
    isError: true,
    searchText: msg.toLowerCase(),
  };
}

function laneForCategory(cat: TraceRowCategory): TimelineLane {
  if (cat === "assistant" || cat === "error") return "model";
  if (cat === "tool") return "tool";
  return "input";
}

export function buildTraceModel(entries: SessionTraceEntry[]): TraceModel {
  const roundMap = new Map<string, TraceRound>();
  const orderedRounds: TraceRound[] = [];
  let lastRoundKey: string | null = null;

  for (const entry of entries) {
    const item = classifyEntry(entry);
    if (!item) continue;

    const key: string = entry.correlation_id || lastRoundKey || "__init__";
    lastRoundKey = key;

    let round = roundMap.get(key);
    if (!round) {
      round = {
        index: orderedRounds.length + 1,
        correlationId: entry.correlation_id ?? null,
        rows: [],
        startedAt: item.startedAt,
        endedAt: item.startedAt + (item.durationMs ?? 0),
        totalDurationMs: 0,
        hasTiming: false,
        totalTokens: 0,
        apiCalls: 0,
        toolCalls: 0,
      };
      roundMap.set(key, round);
      orderedRounds.push(round);
    }

    round.rows.push(item);
    round.startedAt = Math.min(round.startedAt, item.startedAt);
    const itemEnd = item.startedAt + (item.durationMs ?? 0);
    round.endedAt = Math.max(round.endedAt, itemEnd);

    if (item.durationMs != null) {
      round.hasTiming = true;
      round.totalDurationMs += item.durationMs;
    }
    if (item.category === "assistant") {
      round.apiCalls += 1;
      round.totalTokens += item.usage?.total_tokens ?? 0;
    }
    if (item.category === "tool") {
      round.toolCalls += 1;
    }
  }

  // Recompute round-level timing if we have start/end bounds
  let totalDurationMs = 0;
  let hasTiming = false;
  let totalTokens = 0;
  let totalApiCalls = 0;
  let totalToolCalls = 0;

  for (const r of orderedRounds) {
    if (r.hasTiming) {
      hasTiming = true;
      totalDurationMs += r.totalDurationMs;
    }
    totalTokens += r.totalTokens;
    totalApiCalls += r.apiCalls;
    totalToolCalls += r.toolCalls;
  }

  // Build timeline segments across all rows
  const allRows = orderedRounds.flatMap((r) => r.rows.map((row) => ({ round: r, row })));
  const timedWeights = allRows.map(({ row }) =>
    row.durationMs != null ? Math.max(row.durationMs, 15) : 40,
  );
  const weightSum = timedWeights.reduce((acc, w) => acc + w, 0) || 1;

  let cursor = 0;
  const segments: TraceTimelineSegment[] = allRows.map(({ round, row }, i) => {
    const w = timedWeights[i];
    const leftPct = (cursor / weightSum) * 100;
    const widthPct = Math.max((w / weightSum) * 100, 0.8);
    cursor += w;
    return {
      rowId: row.id,
      roundIndex: round.index,
      lane: laneForCategory(row.category),
      category: row.category,
      isError: row.isError,
      label: row.toolName || row.model || row.summary || row.category,
      durationMs: row.durationMs,
      leftPct,
      widthPct,
    };
  });

  return {
    rounds: orderedRounds,
    segments,
    totalDurationMs,
    hasTiming,
    totalTokens,
    totalApiCalls,
    totalToolCalls,
  };
}

export function filterTraceRounds(
  rounds: TraceRound[],
  query: string,
  showCalls: boolean,
): TraceRound[] {
  const q = query.trim().toLowerCase();
  return rounds
    .map((round) => {
      const rows = round.rows.filter((row) => {
        if (!showCalls && (row.category === "tool" || (row.category === "assistant" && !row.entry.data?.text))) {
          return false;
        }
        if (!q) return true;
        return row.searchText.includes(q);
      });
      return { ...round, rows };
    })
    .filter((r) => r.rows.length > 0);
}

export function formatDurationMs(ms: number | null | undefined): string {
  if (ms == null || Number.isNaN(ms)) return "";
  if (ms < 1000) return `${Math.round(ms)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}
