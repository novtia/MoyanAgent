import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../../api/tauri";
import { countWords } from "../../../store/reader";
import type { AssistantBlock } from "../../../types";
import { parseWebFetchOutput } from "./parsers";
import { FlowHead } from "./ToolHeaderRow";
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

  return (
    <div className="web-fetch-tool">
      <FlowHead
        tool="WebFetch"
        name="WebFetch"
        meta={meta || url}
        status={status}
      />
      {url && status === "success" && (
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
      {errorMessage && (
        <div className="tool-call-error-detail" role="alert">
          <span className="tool-call-error-detail-text">{errorMessage}</span>
        </div>
      )}
      {parsed?.text && (
        <button
          type="button"
          className={`fetch-body${open ? " is-open" : ""}`}
          onClick={() => setOpen((v) => !v)}
          title={t("message.toolCallToggle")}
        >
          {parsed.text}
        </button>
      )}
    </div>
  );
}
