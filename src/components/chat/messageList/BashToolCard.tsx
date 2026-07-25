import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantBlock } from "../../../types";
import { parseBashOutput } from "./parsers";
import { FlowHead } from "./ToolHeaderRow";
import { extractToolErrorMessage, summarizeToolInput } from "./utils";

export function BashToolCard({
  block,
}: {
  block: Extract<AssistantBlock, { type: "tool_use" }>;
}) {
  const { t } = useTranslation();
  const status = block.status;
  const input = (block.input ?? {}) as { command?: string; cwd?: string };
  const parsed = useMemo(() => parseBashOutput(block.output), [block.output]);
  const command =
    parsed?.command || input.command || summarizeToolInput(block.input) || "";
  const meta = input.cwd || undefined;
  const errorMessage =
    status === "error" && !parsed
      ? extractToolErrorMessage(block.output)
      : "";

  const exit = parsed?.exit_code;
  const ok = exit === 0;
  const truncated =
    parsed?.stdout_truncated || parsed?.stderr_truncated
      ? t("message.toolOutputTruncated")
      : "";

  return (
    <div className="bash-tool">
      <FlowHead
        tool="Bash"
        name="Bash"
        meta={meta || (command.length > 48 ? undefined : command)}
        status={status}
      />
      <div className="term">
        <div className="term-head">
          <span>shell</span>
          {exit != null && (
            <span className={`exit ${ok ? "ok" : "bad"}`}>
              exit {exit}
            </span>
          )}
          {status === "pending" && (
            <span className="exit">{t("message.toolCallRunning")}</span>
          )}
        </div>
        {command && (
          <div className="term-cmd">
            <span className="ps1">› </span>
            {command}
          </div>
        )}
        {(parsed?.stdout || parsed?.stderr || errorMessage) && (
          <div className="term-out">
            {parsed?.stdout}
            {parsed?.stderr ? (
              <span className="stderr">
                {parsed.stdout ? "\n" : ""}
                {parsed.stderr}
              </span>
            ) : null}
            {!parsed && errorMessage ? (
              <span className="stderr">{errorMessage}</span>
            ) : null}
          </div>
        )}
        {truncated && <div className="term-trunc">{truncated}</div>}
      </div>
    </div>
  );
}
