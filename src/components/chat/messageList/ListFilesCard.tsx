import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import type { ListFilesEntry } from "./types";
import { parseListFilesToolOutput } from "./parsers";
import { ToolHeaderRow } from "./ToolHeaderRow";
import { extractToolErrorMessage } from "./utils";

function TreeView({ entries }: { entries: ListFilesEntry[] }) {
  const { t } = useTranslation();
  return (
    <ul>
      {entries.map((entry, i) => {
        const isDir = entry.kind === "directory";
        return (
          <li key={`${entry.name}:${i}`}>
            {isDir ? (
              <>
                <span className="dir">{entry.name}/</span>
                {entry.children && entry.children.length > 0 && (
                  <TreeView entries={entry.children} />
                )}
              </>
            ) : (
              <>
                {entry.name}
                {entry.paragraphs != null && (
                  <span className="paras">
                    {t("message.listFilesParagraphs", {
                      count: entry.paragraphs,
                    })}
                  </span>
                )}
              </>
            )}
          </li>
        );
      })}
    </ul>
  );
}

function countEntries(entries: ListFilesEntry[]): number {
  let n = 0;
  for (const e of entries) {
    n += 1;
    if (e.children) n += countEntries(e.children);
  }
  return n;
}

export function ListFilesCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(block.status === "success");
  const status = block.status;
  const parsed = useMemo(
    () => parseListFilesToolOutput(block.output),
    [block.output],
  );
  const inputPath =
    block.input && typeof block.input === "object"
      ? String((block.input as { path?: string }).path || "")
      : "";
  const root = parsed?.path || inputPath || ".";
  const baseName = root.split(/[\\/]/).filter(Boolean).pop() || root;
  const count = parsed ? countEntries(parsed.entries) : 0;
  const metaParts = [
    count > 0 ? t("message.listFilesCount", { count }) : "",
    parsed?.truncated ? t("message.toolOutputTruncated") : "",
  ].filter(Boolean);
  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";
  const hasDetail = !!parsed && parsed.entries.length > 0;

  return (
    <ToolHeaderRow
      tool="ListFiles"
      name={baseName.endsWith("/") || baseName.includes(".") ? baseName : `${baseName}/`}
      meta={metaParts.join(" · ")}
      status={status}
      open={open && hasDetail}
      expandable={hasDetail}
      onToggle={() => setOpen((v) => !v)}
      errorMessage={errorMessage || undefined}
      errorLabel={t("message.toolCallErrorReason")}
    >
      <div className="tree">
        <div className="dir">{root.endsWith("/") || root.endsWith("\\") ? root : `${root}/`}</div>
        {parsed && <TreeView entries={parsed.entries} />}
      </div>
    </ToolHeaderRow>
  );
}
