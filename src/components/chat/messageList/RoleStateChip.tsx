import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";

export function RoleStateChip({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const input = (block.input ?? {}) as {
    action?: string;
    id?: string;
    role?: { name?: string };
  };
  const output = (block.output ?? {}) as { role?: { name?: string }; id?: string };
  const action = input.action ?? "";
  const name =
    output.role?.name || input.role?.name || output.id || input.id || "";

  const opLabel =
    action === "create"
      ? t("roleState.opCreate")
      : action === "update"
        ? t("roleState.opUpdate")
        : action === "delete"
          ? t("roleState.opDelete")
          : t("roleState.opRead");

  return (
    <div className="rs-inline-chip">
      <span className="rs-inline-chip-icon">🎭</span>
      <span className="rs-inline-chip-op">{opLabel}</span>
      {name && action !== "get" ? <span>{name}</span> : null}
    </div>
  );
}
