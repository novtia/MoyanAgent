import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatReadToolTitle } from "../../../store/reader";
import type { AssistantBlock } from "../../../types";
import { ToolHeaderRow } from "./ToolHeaderRow";
import { extractToolErrorMessage } from "./utils";

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
  const meta = useMemo(() => {
    const o =
      block.output && typeof block.output === "object"
        ? (block.output as Record<string, unknown>)
        : {};
    const parts: string[] = [];
    if (typeof o.chars === "number" && o.chars > 0) {
      parts.push(`${o.chars}${t("message.createDocCharsUnit")}`);
    } else if (bodyText) {
      parts.push(`${bodyText.length}${t("message.createDocCharsUnit")}`);
    }
    return parts.join(" · ");
  }, [block.output, bodyText, t]);

  const hasDetail = bodyText.length > 0;
  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";

  return (
    <ToolHeaderRow
      tool="Read"
      name={title || t("message.readToolUntitled")}
      meta={meta}
      status={status}
      bare
      open={open && hasDetail}
      expandable={hasDetail}
      onToggle={() => setOpen((v) => !v)}
      errorMessage={errorMessage || undefined}
      errorLabel={t("message.toolCallErrorReason")}
    >
      <pre className="doc-prose doc-prose--scroll">{bodyText}</pre>
    </ToolHeaderRow>
  );
}
