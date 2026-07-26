import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { countWords, resolveToolFilePath } from "../../../store/reader";
import type { AssistantBlock } from "../../../types";
import { normalizeToolContent } from "../../../utils/normalizeToolContent";
import { parseWriteOutput } from "./parsers";
import { ToolHeaderRow } from "./ToolHeaderRow";
import { extractToolErrorMessage } from "./utils";

export function StreamingDocCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const status = block.status;
  const isEdit = block.tool === "Edit";
  const isWrite = block.tool === "Write";
  const isCreateDoc = block.tool === "CreateDoc";
  const streaming = block.streaming === true;
  const [open, setOpen] = useState(status === "pending" || streaming);
  /** Full written content under the truncated preview (CreateDoc / Write). */
  const [contentExpanded, setContentExpanded] = useState(false);
  const [truncated, setTruncated] = useState(false);
  const proseRef = useRef<HTMLDivElement | null>(null);

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
    chars?: number;
    lines?: number;
  };
  const writeOut = isWrite ? parseWriteOutput(block.output) : null;

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

  const content = useMemo(
    () =>
      normalizeToolContent(
        isEdit
          ? typeof output.new_string === "string"
            ? output.new_string
            : (input.new_string ?? "")
          : isWrite
            ? (typeof writeOut?.text === "string" && status === "success"
                ? writeOut.text
                : (input.content ?? ""))
            : (input.content ?? ""),
      ),
    [
      isEdit,
      isWrite,
      output.new_string,
      input.new_string,
      input.content,
      writeOut?.text,
      status,
    ],
  );

  const path = resolveToolFilePath(block.input, block.output);
  const baseName = path ? path.split(/[\\/]/).pop() || path : "";
  const replaceAll = output.replace_all === true || input.replace_all === true;

  const name = useMemo(() => {
    if (isEdit) return baseName || t("message.streamDocEditUntitled");
    if (isWrite) return baseName || t("message.streamDocEditUntitled");
    return (
      (output.title || input.title || "").trim() || t("message.createDocUntitled")
    );
  }, [isEdit, isWrite, baseName, output.title, input.title, t]);

  const meta = useMemo(() => {
    if (isEdit) {
      const parts: string[] = [];
      if (streaming && status === "pending") {
        parts.push(t("message.streamDocEditing"));
      } else if (status === "success") {
        parts.push(t("message.streamDocEdited"));
      }
      if (typeof output.replaced_count === "number") {
        parts.push(
          t("message.streamDocReplaced", { count: output.replaced_count }),
        );
      } else if (replaceAll) {
        parts.push("×N");
      }
      return parts.filter(Boolean).join(" · ");
    }
    if (isWrite) {
      if (streaming && status === "pending") return t("message.streamDocWriting");
      const parts: string[] = [];
      if (status === "success") {
        parts.push(
          writeOut?.created === false
            ? t("message.createDocUpdated")
            : t("message.streamDocWritten"),
        );
      }
      const chars = writeOut?.chars ?? (content ? countWords(content) : 0);
      const lines = writeOut?.lines;
      if (chars > 0) {
        parts.push(`${chars.toLocaleString()}${t("message.createDocCharsUnit")}`);
      }
      if (lines != null && lines > 0) {
        parts.push(`${lines}${t("message.createDocLinesUnit")}`);
      }
      return parts.join(" · ");
    }
    // CreateDoc
    if (streaming && status === "pending") return t("message.createDocWriting");
    if (path) return path;
    return "";
  }, [
    isEdit,
    isWrite,
    streaming,
    status,
    output.replaced_count,
    replaceAll,
    writeOut,
    content,
    path,
    t,
  ]);

  const added = useMemo(() => countWords(content), [content]);
  const removed = useMemo(
    () => (isEdit ? countWords(oldString) : 0),
    [isEdit, oldString],
  );

  const hasContent = content.length > 0 || oldString.length > 0;
  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";

  useEffect(() => {
    if (streaming) setOpen(true);
  }, [streaming]);

  useEffect(() => {
    if (status === "error" && hasContent) setOpen(true);
  }, [status, hasContent]);

  useEffect(() => {
    if (!open) setContentExpanded(false);
  }, [open]);

  const showProseExpand =
    !isEdit && (isCreateDoc || isWrite) && open && hasContent;

  useEffect(() => {
    if (!showProseExpand || contentExpanded) {
      setTruncated(false);
      return;
    }
    const el = proseRef.current;
    if (!el) return;
    const measure = () => {
      setTruncated(el.scrollHeight > el.clientHeight + 2);
    };
    measure();
    const ro = typeof ResizeObserver !== "undefined"
      ? new ResizeObserver(measure)
      : null;
    ro?.observe(el);
    return () => ro?.disconnect();
  }, [showProseExpand, contentExpanded, content]);

  const showExpandBtn = showProseExpand && (truncated || contentExpanded);

  return (
    <ToolHeaderRow
      tool={block.tool}
      name={name}
      meta={meta}
      status={status}
      streaming={streaming}
      open={open && hasContent}
      expandable={hasContent}
      onToggle={() => setOpen((v) => !v)}
      errorMessage={errorMessage || undefined}
      errorLabel={t("message.toolCallErrorReason")}
      footer={
        showExpandBtn ? (
          <button
            type="button"
            className={`doc-prose-expand${contentExpanded ? " is-expanded" : ""}`}
            aria-expanded={contentExpanded}
            title={
              contentExpanded
                ? t("message.createDocCollapseContent")
                : t("message.createDocExpandContent")
            }
            onClick={() => setContentExpanded((v) => !v)}
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
              aria-hidden
            >
              <polyline points="6 9 12 15 18 9" />
            </svg>
          </button>
        ) : null
      }
      tail={
        (added > 0 || removed > 0) && (
          <span className="tool-call-diff-chips" aria-hidden>
            {removed > 0 && (
              <span className="chip del">
                −{removed}
                {t("message.createDocCharsUnit")}
              </span>
            )}
            {added > 0 && (
              <span className="chip add">
                +{added}
                {t("message.createDocCharsUnit")}
              </span>
            )}
          </span>
        )
      }
    >
      {isEdit ? (
        <div className="diff-view">
          {oldString.length > 0 && (
            <div className="diff-line del">
              <span className="gutter">−</span>
              <span className="txt">
                {oldString}
                {streaming && !content && (
                  <span className="stream-doc-cursor" aria-hidden />
                )}
              </span>
            </div>
          )}
          {content.length > 0 && (
            <div className="diff-line add">
              <span className="gutter">+</span>
              <span className="txt">
                {content}
                {streaming && <span className="stream-doc-cursor" aria-hidden />}
              </span>
            </div>
          )}
        </div>
      ) : (
        <div
          ref={proseRef}
          className={`doc-prose${contentExpanded ? " is-expanded" : ""}`}
        >
          {content}
          {streaming && <span className="stream-doc-cursor" aria-hidden />}
        </div>
      )}
    </ToolHeaderRow>
  );
}
