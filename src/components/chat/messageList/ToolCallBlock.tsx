import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { safeJsonStringify, summarizeToolInput } from "./utils";

/** Unknown-tool fallback: header row + JSON dump (no card chrome). */
export function ToolCallBlock({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
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

  return (
    <div className={`generic-tool${open ? " is-open" : ""}`}>
      <button
        type="button"
        className="generic"
        aria-expanded={open}
        title={t("message.toolCallToggle")}
        onClick={() => hasDetail && setOpen((v) => !v)}
        disabled={!hasDetail}
      >
        <span className="n">{block.tool}</span>
        {summary && <span className="a">{summary}</span>}
      </button>
      {open && hasDetail && (
        <div className="generic-pre">
          {inputJson && (
            <>
              {t("message.toolCallInput")}
              {"\n"}
              {inputJson}
              {outputJson ? "\n\n" : ""}
            </>
          )}
          {outputJson && (
            <>
              {t("message.toolCallOutput")}
              {"\n"}
              {outputJson}
            </>
          )}
        </div>
      )}
    </div>
  );
}
