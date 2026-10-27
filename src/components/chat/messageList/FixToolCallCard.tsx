import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { sanitizeFsPath } from "../../../utils/sanitizePath";
import { extractToolErrorMessage } from "./utils";
import { ToolHeaderRow } from "./ToolHeaderRow";

function setFieldNames(input: unknown): string {
  if (!input || typeof input !== "object") return "";
  const set = (input as { set?: unknown }).set;
  if (!set || typeof set !== "object" || Array.isArray(set)) return "";
  return Object.keys(set).join(", ");
}

export function FixToolCallCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const status = block.status;
  const output = (block.output ?? {}) as {
    amended_tool?: string;
    title?: string;
    path?: string;
    name?: string;
    chars?: number;
  };
  const toolName = (output.amended_tool || "").trim();
  const title = (output.title || output.name || "").trim();
  const path = sanitizeFsPath(output.path || "");
  const base = path.split(/[\\/]/).filter(Boolean).pop() || "";
  const fields = setFieldNames(block.input);

  const headline =
    status === "pending"
      ? t("message.fixToolCallRunning")
      : title ||
        base ||
        toolName ||
        (status === "error"
          ? t("message.fixToolCallFailed")
          : t("message.fixToolCallUnknown"));

  const metaParts: string[] = [];
  if (fields) metaParts.push(fields);
  if (base && base !== headline) metaParts.push(base);
  if (typeof output.chars === "number" && output.chars > 0) {
    metaParts.push(
      `${output.chars.toLocaleString()}${t("message.createDocCharsUnit")}`,
    );
  }
  if (status === "success") {
    metaParts.push(
      t("message.fixToolCallDone", {
        tool: toolName || t("message.fixToolCallUnknown"),
      }),
    );
  } else if (status === "error" && headline !== t("message.fixToolCallFailed")) {
    metaParts.push(t("message.fixToolCallFailed"));
  } else if (status === "pending" && toolName) {
    metaParts.push(toolName);
  }

  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";

  return (
    <ToolHeaderRow
      tool="FixToolCall"
      name={headline}
      meta={metaParts.join(" · ")}
      status={status}
      streaming={block.streaming === true}
      errorMessage={errorMessage || undefined}
      errorLabel={t("message.toolCallErrorReason")}
    />
  );
}
