import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { useSession } from "../../../store/session";
import { parseAgentOutput } from "./parsers";
import { ToolGlyph } from "./toolIcons";
import { extractToolErrorMessage } from "./utils";

export function AgentToolCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const switchTo = useSession((s) => s.switchTo);
  const status = block.status;
  const input = (block.input ?? {}) as {
    description?: string;
    prompt?: string;
    subagent_type?: string;
    run_in_background?: boolean;
    name?: string;
  };
  const parsed = useMemo(() => parseAgentOutput(block.output), [block.output]);
  const title =
    input.description || input.name || input.prompt?.slice(0, 60) || "Agent";

  const childSessionId =
    parsed && "child_session_id" in parsed ? parsed.child_session_id : undefined;

  const errorMessage =
    status === "error" ? extractToolErrorMessage(block.output) : "";

  const running =
    status === "pending" ||
    parsed?.status === "running" ||
    (parsed?.status === "async_launched" && status !== "error");

  const failed = status === "error";

  const statusLabel = failed
    ? t("message.agentCardFailed")
    : running
      ? t("message.agentCardRunning")
      : t("message.agentCardCompleted");

  const canOpen = !!childSessionId;

  const onOpen = () => {
    if (!childSessionId) return;
    void switchTo(childSessionId);
  };

  return (
    <button
      type="button"
      className={`agent-card${running ? " is-running" : ""}${failed ? " is-error" : ""}`}
      onClick={onOpen}
      disabled={!canOpen}
      title={canOpen ? t("message.agentCardOpenHint") : undefined}
    >
      <span className="agent-card-icon" aria-hidden>
        <ToolGlyph tool="Agent" />
      </span>
      <span className="agent-card-main">
        <span className="agent-card-title">{title}</span>
        <span className="agent-card-status">
          {errorMessage || statusLabel}
        </span>
      </span>
      {canOpen && (
        <span className="agent-card-hint">{t("message.agentCardOpen")}</span>
      )}
    </button>
  );
}
