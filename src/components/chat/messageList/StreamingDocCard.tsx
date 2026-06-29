import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { countChars, formatParagraphRangeLabel, parseEditParagraphRange, readerDocFromToolOutput, useReader } from "../../../store/reader";
import type { AssistantBlock } from "../../../types";
import { ThinkingChevronIcon, ToolCallIcon } from "./icons";

export function StreamingDocCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const status = block.status;
  const isEdit = block.tool === "Edit";
  const streaming = block.streaming === true;
  const [open, setOpen] = useState(status === "pending" || streaming);

  const input = (block.input ?? {}) as {
    title?: string;
    doc_type?: string;
    content?: string;
    path?: string;
    paragraph_from?: number;
    paragraph_to?: number;
  };
  const output = (block.output ?? {}) as {
    title?: string;
    path?: string;
    created?: boolean;
  };

  const content = isEdit ? input.content ?? "" : input.content ?? "";

  const path = (output.path || input.path || "").toString();
  const baseName = path ? path.split(/[\\/]/).pop() || path : "";
  const editRange = isEdit ? parseEditParagraphRange(input) : undefined;
  const paraLabel =
    editRange != null
      ? formatParagraphRangeLabel(editRange.from, editRange.to)
      : "";
  const summary = isEdit
    ? [baseName || t("message.streamDocEditUntitled"), paraLabel].filter(Boolean).join(" · ")
    : (output.title || input.title || "").trim() ||
      t("message.createDocUntitled");

  const added = useMemo(() => countChars(content), [content]);

  const readerDoc = useMemo(
    () =>
      !isEdit && status === "success"
        ? readerDocFromToolOutput(block.output)
        : null,
    [isEdit, block.output, status],
  );

  const statusLabel =
    status === "pending"
      ? t("message.toolCallRunning")
      : status === "error"
        ? t("message.toolCallError")
        : t("message.toolCallDone");

  const hasContent = content.length > 0;

  useEffect(() => {
    if (streaming) setOpen(true);
  }, [streaming]);

  return (
    <div
      className={`tool-call-block ${status} ${open ? "is-open" : ""} ${
        streaming ? "is-streaming" : ""
      }`}
    >
      <button
        type="button"
        className="tool-call-summary"
        aria-expanded={open}
        title={t("message.toolCallToggle")}
        onClick={() => hasContent && setOpen((v) => !v)}
        disabled={!hasContent}
      >
        <ToolCallIcon status={status} />
        <span className="tool-call-name">{block.tool}</span>
        {summary && <span className="tool-call-args">{summary}</span>}
        <span className="tool-call-spacer" aria-hidden />
        {added > 0 && (
          <span className="tool-call-diff-chips" aria-hidden={added === 0}>
            <span className="is-add">
              +{added}
              {t("message.createDocCharsUnit")}
            </span>
          </span>
        )}
        {readerDoc && (
          <span
            className="tool-call-read-btn"
            role="button"
            tabIndex={0}
            title={t("message.openInReader")}
            onClick={(e) => {
              e.stopPropagation();
              useReader.getState().openDoc(readerDoc);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                e.stopPropagation();
                useReader.getState().openDoc(readerDoc);
              }
            }}
          >
            {t("message.openInReader")}
          </span>
        )}
        <span className={`tool-call-badge ${status}`}>{statusLabel}</span>
        {hasContent && <ThinkingChevronIcon />}
      </button>

      {open && hasContent && (
        <div className="tool-call-detail">
          <pre className="tool-call-detail-body">
            {content}
            {streaming && <span className="stream-doc-cursor" aria-hidden />}
          </pre>
        </div>
      )}
    </div>
  );
}
