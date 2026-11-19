import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TraceRowItem } from "./traceModel";
import { formatDurationMs } from "./traceModel";

interface TraceRowProps {
  row: TraceRowItem;
  showDuration: boolean;
  highlighted: boolean;
  defaultExpanded?: boolean;
}

function prettyJson(v: unknown): string {
  if (v == null) return "";
  if (typeof v === "string") return v;
  try {
    return JSON.stringify(v, null, 2);
  } catch {
    return String(v);
  }
}

export function TraceRow({
  row,
  showDuration,
  highlighted,
  defaultExpanded = false,
}: TraceRowProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(defaultExpanded);
  const { entry } = row;
  const data = entry.data ?? {};
  const durStr = showDuration ? formatDurationMs(row.durationMs) : "";

  const totalTokens = row.usage?.total_tokens;
  const agentType = entry.agent_type && entry.agent_type !== "main" ? entry.agent_type : null;

  return (
    <div
      id={`trace-row-${row.id}`}
      className={`trace-row is-${row.category}${row.isError ? " is-error" : ""}${highlighted ? " is-highlighted" : ""}${expanded ? " is-expanded" : ""}`}
    >
      <button
        type="button"
        className="trace-row-head"
        onClick={() => setExpanded((v) => !v)}
        aria-expanded={expanded}
      >
        <span className={`trace-badge is-${row.category}${row.isError ? " is-error" : ""}`}>
          {t(row.badgeLabelKey)}
        </span>
        {agentType && <span className="trace-chip-agent">{agentType}</span>}
        <span className="trace-row-summary">{row.summary || "—"}</span>
        <span className="trace-row-meta">
          {totalTokens != null && (
            <span className="trace-meta-pill">{totalTokens.toLocaleString()} tok</span>
          )}
          {durStr && <span className="trace-meta-pill is-dur">{durStr}</span>}
        </span>
      </button>

      {expanded && (
        <div className="trace-row-detail">
          <div className="trace-detail-grid">
            <span>{t("trace.detail.time")}</span>
            <span>{new Date(row.startedAt).toLocaleTimeString()}</span>
            {entry.agent_type && (
              <>
                <span>{t("trace.detail.agent")}</span>
                <span>{entry.agent_type}</span>
              </>
            )}
            {row.model && (
              <>
                <span>{t("trace.detail.model")}</span>
                <span>
                  {row.provider ? `${row.provider} / ` : ""}
                  {row.model}
                </span>
              </>
            )}
            {row.turnIndex != null && (
              <>
                <span>{t("trace.detail.step")}</span>
                <span>#{row.turnIndex}</span>
              </>
            )}
            {row.durationMs != null && (
              <>
                <span>{t("trace.detail.duration")}</span>
                <span>{formatDurationMs(row.durationMs)}</span>
              </>
            )}
            {row.usage && (
              <>
                <span>{t("trace.detail.tokens")}</span>
                <span>
                  in {row.usage.prompt_tokens ?? 0} / out {row.usage.completion_tokens ?? 0}
                  {row.usage.cache_read_tokens ? ` / cache ${row.usage.cache_read_tokens}` : ""}
                  {row.usage.total_tokens != null ? ` (total ${row.usage.total_tokens})` : ""}
                </span>
              </>
            )}
          </div>

          {entry.kind === "session_settings" && typeof data.system_prompt === "string" && (
            <div className="trace-detail-block">
              <div className="trace-detail-label">{t("trace.detail.systemPrompt")}</div>
              <pre className="trace-detail-pre">{data.system_prompt}</pre>
            </div>
          )}

          {entry.kind === "context" && typeof data.content === "string" && (
            <div className="trace-detail-block">
              <div className="trace-detail-label">
                {typeof data.label === "string" ? data.label : t("trace.badge.context")}
                {data.truncated ? ` (${t("trace.detail.truncated")})` : ""}
              </div>
              <pre className="trace-detail-pre">{data.content}</pre>
            </div>
          )}

          {entry.kind === "user_message" && typeof data.text === "string" && (
            <div className="trace-detail-block">
              <div className="trace-detail-label">{t("trace.detail.prompt")}</div>
              <pre className="trace-detail-pre">{data.text}</pre>
            </div>
          )}

          {entry.kind === "assistant_turn" && (
            <>
              {typeof data.thinking === "string" && data.thinking && (
                <div className="trace-detail-block">
                  <div className="trace-detail-label">{t("trace.detail.thinking")}</div>
                  <pre className="trace-detail-pre">{data.thinking}</pre>
                </div>
              )}
              {typeof data.text === "string" && data.text && (
                <div className="trace-detail-block">
                  <div className="trace-detail-label">{t("trace.detail.reply")}</div>
                  <pre className="trace-detail-pre">{data.text}</pre>
                </div>
              )}
              {Array.isArray(data.tool_calls) && data.tool_calls.length > 0 && (
                <div className="trace-detail-block">
                  <div className="trace-detail-label">{t("trace.detail.toolCalls")}</div>
                  <pre className="trace-detail-pre">{data.tool_calls.join(", ")}</pre>
                </div>
              )}
            </>
          )}

          {entry.kind === "tool_call" && (
            <>
              {data.input !== undefined && (
                <div className="trace-detail-block">
                  <div className="trace-detail-label">{t("trace.detail.input")}</div>
                  <pre className="trace-detail-pre">{prettyJson(data.input)}</pre>
                </div>
              )}
              {(data.output !== undefined || data.error !== undefined) && (
                <div className="trace-detail-block">
                  <div className="trace-detail-label">
                    {data.is_error ? t("trace.detail.error") : t("trace.detail.output")}
                    {data.truncated ? ` (${t("trace.detail.truncated")})` : ""}
                  </div>
                  <pre className={`trace-detail-pre${data.is_error ? " is-error" : ""}`}>
                    {prettyJson(data.is_error ? (data.error ?? data.output) : data.output)}
                  </pre>
                </div>
              )}
            </>
          )}

          {entry.kind === "error" && (
            <div className="trace-detail-block">
              <div className="trace-detail-label">{t("trace.detail.error")}</div>
              <pre className="trace-detail-pre is-error">
                {typeof data.message === "string" ? data.message : prettyJson(data)}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
