import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api/tauri";
import type { AssistantBlock } from "../../../types";
import { parseWebSearchOutput } from "./parsers";
import { ToolGlyph } from "./toolIcons";
import { extractToolErrorMessage } from "./utils";

export function WebSearchToolCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const status = block.status;
  const input = (block.input ?? {}) as { query?: string };
  const parsed = useMemo(
    () => parseWebSearchOutput(block.output),
    [block.output],
  );
  const query = parsed?.query || input.query || "";
  const hitCount = parsed?.hits.length ?? 0;
  const meta = [
    parsed?.backend,
    query ? `"${query}"` : "",
    status === "success" && hitCount > 0
      ? t("message.webSearchHitCount", { count: hitCount })
      : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";
  const hasDetail =
    !!errorMessage ||
    (!!parsed && parsed.hits.length > 0) ||
    (status === "success" && !!parsed && parsed.hits.length === 0);

  return (
    <div className={`web-search-tool${open ? " is-open" : ""}`}>
      <button
        type="button"
        className="flow-head"
        aria-expanded={hasDetail ? open : undefined}
        title={hasDetail ? t("message.toolCallToggle") : undefined}
        onClick={() => hasDetail && setOpen((v) => !v)}
        disabled={!hasDetail}
      >
        <span className="ti">
          <ToolGlyph tool="WebSearch" />
        </span>
        <span className="t">WebSearch</span>
        {meta && <span className="m">{meta}</span>}
      </button>
      {open && (
        <div className="web-search-body">
          {errorMessage && (
            <div className="tool-call-error-detail" role="alert">
              <span className="tool-call-error-detail-text">{errorMessage}</span>
            </div>
          )}
          {parsed && parsed.hits.length > 0 && (
            <div className="hits">
              {parsed.hits.map((hit, i) => (
                <button
                  type="button"
                  className="hit"
                  key={`${hit.url}:${i}`}
                  onClick={() => {
                    if (hit.url) api.openUrl(hit.url).catch(console.warn);
                  }}
                >
                  <div className="hit-title">{hit.title || hit.url}</div>
                  {hit.snippet && (
                    <div className="hit-snippet">{hit.snippet}</div>
                  )}
                  {hit.url && (
                    <div className="hit-url">
                      <svg
                        width="10"
                        height="10"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        aria-hidden
                      >
                        <path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6" />
                        <polyline points="15 3 21 3 21 9" />
                        <line x1="10" y1="14" x2="21" y2="3" />
                      </svg>
                      {hit.url.replace(/^https?:\/\//, "")}
                    </div>
                  )}
                </button>
              ))}
            </div>
          )}
          {status === "success" && parsed && parsed.hits.length === 0 && (
            <div className="kv" style={{ padding: "4px 2px" }}>
              {parsed.message || t("message.webSearchEmpty")}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
