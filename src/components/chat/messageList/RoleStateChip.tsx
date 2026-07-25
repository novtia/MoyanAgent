import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { ToolGlyph } from "./toolIcons";

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
    set?: Record<string, unknown>;
    unset?: string[];
  };
  const output = (block.output ?? {}) as {
    role?: { name?: string };
    id?: string;
    op?: string;
  };
  const action = input.action ?? output.op ?? "";
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

  const changeBits: string[] = [];
  if (input.set && typeof input.set === "object") {
    for (const [k, v] of Object.entries(input.set).slice(0, 3)) {
      changeBits.push(`${k}: ${typeof v === "string" ? v : JSON.stringify(v)}`);
    }
  }
  if (Array.isArray(input.unset) && input.unset.length > 0) {
    changeBits.push(`−${input.unset.slice(0, 3).join(", ")}`);
  }
  const summary = changeBits.join(" · ");

  return (
    <div className="rs-inline-chip">
      <span className="ti" aria-hidden>
        <ToolGlyph tool="RoleState" />
      </span>
      <span className="op">{opLabel}</span>
      {name && action !== "get" ? <b>{name}</b> : null}
      {summary ? <span className="rs-inline-chip-summary">· {summary}</span> : null}
    </div>
  );
}
