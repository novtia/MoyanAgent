import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { sanitizeFsPath } from "../../../utils/sanitizePath";
import { ToolGlyph } from "./toolIcons";

function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? "";
}

type SplitPart = {
  name?: string;
  path?: string;
  chars?: number;
};

export function SplitDocCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const status = block.status;
  const input = (block.input ?? {}) as { path?: string };
  const output = (block.output ?? {}) as { path?: string; parts?: SplitPart[] };
  const parts = Array.isArray(output.parts) ? output.parts : [];

  const sourcePath = sanitizeFsPath(output.path || input.path || "");
  const sourceName =
    baseName(sourcePath) ||
    (parts[0]?.name || "").trim() ||
    t("message.splitDocUntitled");

  const label =
    status === "pending"
      ? t("message.splitDocSplitting")
      : status === "error"
        ? t("message.splitDocFailed")
        : t("message.splitDocSplit");

  return (
    <div className={`rename-line split-line${status === "error" ? " is-error" : ""}`}>
      <span className="ti">
        <ToolGlyph tool="SplitDoc" />
      </span>
      <span className="fname" title={sourcePath || sourceName}>
        {sourceName}
      </span>
      {parts.length > 0 && (
        <span className="arrow" aria-hidden>
          →
        </span>
      )}
      {parts.map((part, index) => {
        const name = (part.name || "").trim() || baseName(sanitizeFsPath(part.path || ""));
        if (!name) return null;
        const chars = typeof part.chars === "number" ? ` · ${part.chars}` : "";
        return (
          <span key={`${name}-${index}`} className="fname" title={sanitizeFsPath(part.path || "") || name}>
            {name}
            {chars}
          </span>
        );
      })}
      <span className="kv">{label}</span>
    </div>
  );
}
