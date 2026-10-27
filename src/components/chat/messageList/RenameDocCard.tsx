import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { sanitizeFsPath } from "../../../utils/sanitizePath";
import { ToolGlyph } from "./toolIcons";

function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? "";
}

export function RenameDocCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const status = block.status;
  const input = (block.input ?? {}) as { path?: string; new_name?: string };
  const output = (block.output ?? {}) as { from?: string; path?: string; name?: string };

  const fromPath = sanitizeFsPath(output.from || input.path || "");
  const toPath = sanitizeFsPath(output.path || "");
  const fromName = baseName(fromPath) || t("message.renameDocUntitled");
  const toName =
    (output.name || "").trim() ||
    baseName(toPath) ||
    (input.new_name || "").trim() ||
    t("message.renameDocUntitled");

  const label =
    status === "pending"
      ? t("message.renameDocRenaming")
      : status === "error"
        ? t("message.renameDocFailed")
        : t("message.renameDocRenamed");

  return (
    <div className={`rename-line${status === "error" ? " is-error" : ""}`}>
      <span className="ti">
        <ToolGlyph tool="Rename" />
      </span>
      <span className="fname" title={fromPath || fromName}>
        {fromName}
      </span>
      <span className="arrow" aria-hidden>
        →
      </span>
      <span className="fname" title={toPath || toName}>
        {toName}
      </span>
      <span className="kv">{label}</span>
    </div>
  );
}
