import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { ToolCallIcon } from "./icons";

export function DeleteDocCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const status = block.status;
  const input = (block.input ?? {}) as { path?: string };
  const output = (block.output ?? {}) as { name?: string; path?: string };

  const fullPath = output.path || input.path || "";
  const name =
    (output.name || "").trim() ||
    fullPath.split(/[\\/]/).filter(Boolean).pop() ||
    t("message.deleteDocUntitled");

  const statusLabel =
    status === "pending"
      ? t("message.toolCallRunning")
      : status === "error"
        ? t("message.toolCallError")
        : t("message.toolCallDone");

  return (
    <div className={`tool-call-block ${status}`}>
      <div className="tool-call-summary tool-call-summary--static">
        <ToolCallIcon status={status} />
        <span className="tool-call-name">Delete</span>
        <span className="tool-call-args" title={fullPath || name}>
          {name}
        </span>
        <span className="tool-call-spacer" aria-hidden />
        <span className={`tool-call-badge ${status}`}>{statusLabel}</span>
      </div>
    </div>
  );
}
