import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { sanitizeFsPath } from "../../../utils/sanitizePath";
import { ToolGlyph } from "./toolIcons";

export function DeleteDocCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const status = block.status;
  const input = (block.input ?? {}) as { path?: string };
  const output = (block.output ?? {}) as { name?: string; path?: string };

  const fullPath = sanitizeFsPath(output.path || input.path || "");
  const name =
    (output.name || "").trim() ||
    fullPath.split(/[\\/]/).filter(Boolean).pop() ||
    t("message.deleteDocUntitled");

  const label =
    status === "pending"
      ? t("message.deleteDocDeleting")
      : status === "error"
        ? t("message.deleteDocFailed")
        : t("message.deleteDocDeleted");

  return (
    <div className={`del-line${status === "error" ? " is-error" : ""}`}>
      <span className="ti">
        <ToolGlyph tool="Delete" />
      </span>
      <span className="fname" title={fullPath || name}>
        {name}
      </span>
      <span className="kv">{label}</span>
    </div>
  );
}
