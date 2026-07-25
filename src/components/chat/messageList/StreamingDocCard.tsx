import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  countWords,
  readerDocFromToolOutput,
  resolveToolFilePath,
  useReader,
} from "../../../store/reader";
import type { AssistantBlock } from "../../../types";
import { normalizeToolContent } from "../../../utils/normalizeToolContent";
import { extractToolErrorMessage } from "./utils";
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
    old_string?: string;
    new_string?: string;
    replace_all?: boolean;
  };
  const output = (block.output ?? {}) as {
    title?: string;
    path?: string;
    created?: boolean;
    old_string?: string;
    new_string?: string;
    replace_all?: boolean;
    replaced_count?: number;
  };

  const oldString = useMemo(
    () =>
      isEdit
        ? normalizeToolContent(
            typeof output.old_string === "string"
              ? output.old_string
              : (input.old_string ?? ""),
          )
        : "",
    [isEdit, output.old_string, input.old_string],
  );

  // For Edit the detail/word-count reflects the replacement text (`new_string`);
  // for CreateDoc it is the written `content`.
  const content = useMemo(
    () =>
      normalizeToolContent(
        isEdit
          ? typeof output.new_string === "string"
            ? output.new_string
            : (input.new_string ?? "")
          : (input.content ?? ""),
      ),
    [isEdit, output.new_string, input.new_string, input.content],
  );

  const path = resolveToolFilePath(block.input, block.output);
  const baseName = path ? path.split(/[\\/]/).pop() || path : "";
  const replaceAll = output.replace_all === true || input.replace_all === true;
  const summary = isEdit
    ? [
        streaming && status === "pending"
          ? t("message.streamDocEditing")
          : null,
        baseName || t("message.streamDocEditUntitled"),
        replaceAll ? "×N" : "",
      ]
        .filter(Boolean)
        .join(" · ")
    : (output.title || input.title || "").trim() ||
      t("message.createDocUntitled");

  const added = useMemo(() => countWords(content), [content]);
  const removed = useMemo(
    () => (isEdit ? countWords(oldString) : 0),
    [isEdit, oldString],
  );

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

  const hasContent = content.length > 0 || oldString.length > 0;
  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";

  useEffect(() => {
    if (streaming) setOpen(true);
  }, [streaming]);

  useEffect(() => {
    if (status === "error" && hasContent) setOpen(true);
  }, [status, hasContent]);

  return (
    <div
      className={`tool-call-block doc-tool-card ${status} ${open ? "is-open" : ""} ${
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
        {(added > 0 || removed > 0) && (
          <span className="tool-call-diff-chips" aria-hidden>
            {removed > 0 && (
              <span className="is-del">
                −{removed}
                {t("message.createDocCharsUnit")}
              </span>
            )}
            {added > 0 && (
              <span className="is-add">
                +{added}
                {t("message.createDocCharsUnit")}
              </span>
            )}
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
          {isEdit ? (
            <div className="tool-call-edit-stream">
              {oldString.length > 0 && (
                <pre className="tool-call-detail-body is-delete">
                  {oldString}
                  {streaming && !content && (
                    <span className="stream-doc-cursor" aria-hidden />
                  )}
                </pre>
              )}
              {content.length > 0 && (
                <pre className="tool-call-detail-body is-insert">
                  {content}
                  {streaming && <span className="stream-doc-cursor" aria-hidden />}
                </pre>
              )}
            </div>
          ) : (
            <pre className="tool-call-detail-body">
              {content}
              {streaming && <span className="stream-doc-cursor" aria-hidden />}
            </pre>
          )}
        </div>
      )}

      {errorMessage && (
        <div className="tool-call-error-detail" role="alert">
          <span className="tool-call-error-detail-label">
            {t("message.toolCallErrorReason")}
          </span>
          <span className="tool-call-error-detail-text">{errorMessage}</span>
        </div>
      )}
    </div>
  );
}
