import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import type { ListFilesEntry } from "./types";
import { parseListFilesOutput, safeJsonStringify, summarizeToolInput } from "./utils";
import { ThinkingChevronIcon, ToolCallIcon } from "./icons";

export function ListFilesTreeView({ entries }: { entries: ListFilesEntry[] }) {
  return (
    <ul className="list-files-tree">
      {entries.map((entry, i) => (
        <ListFilesTreeNode key={`${entry.name}:${i}`} entry={entry} />
      ))}
    </ul>
  );
}

export function ListFilesTreeNode({ entry }: { entry: ListFilesEntry }) {
  const { t } = useTranslation();
  const isDir = entry.kind === "directory";
  const children = entry.children ?? [];
  return (
    <li className={`list-files-tree-node ${isDir ? "is-dir" : "is-file"}`}>
      <span className="list-files-tree-label">
        <span className="list-files-tree-kind" aria-hidden>
          {isDir ? "▸" : "·"}
        </span>
        {entry.name}
        {!isDir && entry.paragraphs != null && (
          <span className="list-files-tree-paragraphs">
            {t("message.listFilesParagraphs", { count: entry.paragraphs })}
          </span>
        )}
      </span>
      {isDir && (
        <ListFilesTreeView entries={children} />
      )}
    </li>
  );
}

export function ToolCallBlock({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const status = block.status;
  const statusLabel =
    status === "pending"
      ? t("message.toolCallRunning")
      : status === "error"
        ? t("message.toolCallError")
        : t("message.toolCallDone");
  const summary = useMemo(() => summarizeToolInput(block.input), [block.input]);
  const hasDetail =
    (block.input !== undefined && block.input !== null) ||
    block.output !== undefined;
  const inputJson = useMemo(
    () => safeJsonStringify(block.input),
    [block.input],
  );
  const outputJson = useMemo(
    () => safeJsonStringify(block.output),
    [block.output],
  );

  const listFilesEntries = useMemo(
    () =>
      block.tool === "ListFiles" && status === "success"
        ? parseListFilesOutput(block.output)
        : null,
    [block.tool, block.output, status],
  );

  return (
    <div
      className={`tool-call-block ${status} ${open ? "is-open" : ""}`}
    >
      <button
        type="button"
        className="tool-call-summary"
        aria-expanded={open}
        title={t("message.toolCallToggle")}
        onClick={() => hasDetail && setOpen((v) => !v)}
        disabled={!hasDetail}
      >
        <ToolCallIcon status={status} />
        <span className="tool-call-name">{block.tool}</span>
        {summary && <span className="tool-call-args">{summary}</span>}
        <span className="tool-call-spacer" aria-hidden />
        <span className={`tool-call-badge ${status}`}>{statusLabel}</span>
        {hasDetail && <ThinkingChevronIcon />}
      </button>
      {open && hasDetail && (
        <div className="tool-call-detail">
          {inputJson && (
            <>
              <div className="tool-call-detail-label">
                {t("message.toolCallInput")}
              </div>
              <pre className="tool-call-detail-body">{inputJson}</pre>
            </>
          )}
          {listFilesEntries ? (
            <>
              <div className="tool-call-detail-label">
                {t("message.toolCallOutput")}
              </div>
              <div className="tool-call-detail-body tool-call-detail-body--tree">
                <ListFilesTreeView entries={listFilesEntries} />
              </div>
            </>
          ) : (
            outputJson && (
              <>
                <div className="tool-call-detail-label">
                  {t("message.toolCallOutput")}
                </div>
                <pre className="tool-call-detail-body">{outputJson}</pre>
              </>
            )
          )}
        </div>
      )}
    </div>
  );
}
