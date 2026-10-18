import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { formatReadToolTitle, parseReadBatch } from "../../../store/reader";
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
  // Batched call (`reads: [...]`): one card, every item's title and text.
  const batch = useMemo(
    () => parseReadBatch(block.input, block.output),
    [block.input, block.output],
  );
  const title = useMemo(() => {
    if (batch) {
      const names = batch.map((b) => b.title).filter(Boolean);
      return names.join(", ");
    }
    return formatReadToolTitle(block.input, block.output);
  }, [batch, block.input, block.output]);
  const bodyText = useMemo(() => {
    if (batch) {
      return batch
        .map((b) => {
          const head = `── ${b.title || "?"} ──`;
          if (b.error) return `${head}\n${b.error}`;
          return b.text ? `${head}\n${b.text}` : head;
        })
        .join("\n\n");
    }
    const o = block.output;
    if (!o || typeof o !== "object") return "";
    const text = (o as { text?: unknown }).text;
    return typeof text === "string" ? text : "";
  }, [batch, block.output]);
  const meta = useMemo(() => {
    const o =
      block.output && typeof block.output === "object"
        ? (block.output as Record<string, unknown>)
        : {};
    const parts: string[] = [];
    if (batch) {
      const chars = batch.reduce((n, b) => n + b.text.length, 0);
      if (batch.length > 1) parts.push(`×${batch.length}`);
      if (chars > 0) parts.push(`${chars}${t("message.createDocCharsUnit")}`);
    } else if (typeof o.chars === "number" && o.chars > 0) {
      parts.push(`${o.chars}${t("message.createDocCharsUnit")}`);
    } else if (bodyText) {
      parts.push(`${bodyText.length}${t("message.createDocCharsUnit")}`);
    }
    return parts.join(" · ");
  }, [batch, block.output, bodyText, t]);

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
