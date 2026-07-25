import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api/tauri";
import { countWords } from "../../../store/reader";
import type { AssistantBlock } from "../../../types";
import { parseWebFetchOutput } from "./parsers";
import { ToolGlyph } from "./toolIcons";
import { extractToolErrorMessage } from "./utils";

export function WebFetchToolCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const status = block.status;
  const input = (block.input ?? {}) as { url?: string };
  const parsed = useMemo(
    () => parseWebFetchOutput(block.output),
    [block.output],
  );
  const url = parsed?.url || input.url || "";
  const title = parsed?.title || url;
  const chars = parsed ? countWords(parsed.text) : 0;
  const meta = [
    title !== url ? title : "",
    chars > 0
      ? `${chars.toLocaleString()}${t("message.createDocCharsUnit")}`
      : "",
    parsed?.truncated ? t("message.toolOutputTruncated") : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";
  const hasDetail =
    !!errorMessage || !!parsed?.text || (!!url && status === "success");

  return (
    <div className={`web-fetch-tool${open ? " is-open" : ""}`}>
      <button
        type="button"
        className="flow-head"
        aria-expanded={hasDetail ? open : undefined}
        title={hasDetail ? t("message.toolCallToggle") : undefined}
        onClick={() => hasDetail && setOpen((v) => !v)}
        disabled={!hasDetail}
      >
        <span className="ti">
          <ToolGlyph tool="WebFetch" />
        </span>
        <span className="t">WebFetch</span>
        {meta && <span className="m">{meta}</span>}
      </button>
      {open && url && status === "success" && (
        <button
          type="button"
          className="hit-url"
          style={{
            appearance: "none",
            border: "none",
            background: "transparent",
            padding: "2px 4px 6px",
            cursor: "pointer",
            textAlign: "left",
          }}
          onClick={() => api.openUrl(url).catch(console.warn)}
        >
          {url.replace(/^https?:\/\//, "")}
        </button>
      )}
      {open && errorMessage && (
        <div className="tool-call-error-detail" role="alert">
          <span className="tool-call-error-detail-text">{errorMessage}</span>
        </div>
      )}
      {open && parsed?.text && (
        <div className="fetch-body">{parsed.text}</div>
      )}
    </div>
  );
}
