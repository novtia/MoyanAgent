import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatReadToolTitle } from "../../../store/reader";
import type { AssistantBlock } from "../../../types";
import { ThinkingChevronIcon, ToolCallIcon } from "./icons";

export function ReadToolCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const status = block.status;
  const title = useMemo(
    () => formatReadToolTitle(block.input, block.output),
    [block.input, block.output],
  );
  const bodyText = useMemo(() => {
    const o = block.output;
    if (!o || typeof o !== "object") return "";
    const text = (o as { text?: unknown }).text;
    return typeof text === "string" ? text : "";
  }, [block.output]);
  const hasDetail = bodyText.length > 0;

  const statusLabel =
    status === "pending"
      ? t("message.toolCallRunning")
      : status === "error"
        ? t("message.toolCallError")
        : t("message.toolCallDone");

  return (
    <div className={`tool-call-block read-tool-card ${status} ${open ? "is-open" : ""}`}>
      <button
        type="button"
        className="tool-call-summary"
        aria-expanded={open}
        title={t("message.toolCallToggle")}
        onClick={() => hasDetail && setOpen((v) => !v)}
        disabled={!hasDetail}
      >
        <ToolCallIcon status={status} />
        <span className="tool-call-name read-tool-title" title={title}>
          {title || t("message.readToolUntitled")}
        </span>
        <span className="tool-call-spacer" aria-hidden />
        <span className={`tool-call-badge ${status}`}>{statusLabel}</span>
        {hasDetail && <ThinkingChevronIcon />}
      </button>
      {open && hasDetail && (
        <div className="tool-call-detail">
          <pre className="tool-call-detail-body">{bodyText}</pre>
        </div>
      )}
    </div>
  );
}
