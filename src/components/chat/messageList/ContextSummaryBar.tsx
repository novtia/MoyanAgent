import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../../store/session";
import { ToolGlyph } from "./toolIcons";

export function ContextSummaryBar({
  text,
  childSessionId,
  status,
}: {
  text: string;
  childSessionId?: string;
  status?: "running" | "completed" | "failed";
}) {
  const { t } = useTranslation();
  const switchTo = useSession((s) => s.switchTo);
  const [open, setOpen] = useState(false);
  const canOpen = !!childSessionId;
  const failed = status === "failed";
  const running = status === "running";
  const statusLabel = failed
    ? t("message.agentCardFailed")
    : running
      ? t("message.agentCardRunning")
      : t("message.agentCardCompleted");

  const onOpen = () => {
    if (!childSessionId) {
      setOpen((value) => !value);
      return;
    }
    void switchTo(childSessionId);
  };

  return (
    <div className={`summary-card${open && !canOpen ? " is-open" : ""}`}>
      <button
        type="button"
        className={`agent-card${running ? " is-running" : ""}${failed ? " is-error" : ""}`}
        aria-expanded={canOpen ? undefined : open}
        title={
          canOpen
            ? t("message.summaryCardOpenHint")
            : open
              ? t("message.summaryCardCollapse")
              : t("message.summaryCardOpenHint")
        }
        onClick={onOpen}
      >
        <span className="agent-card-icon" aria-hidden>
          <ToolGlyph tool="Agent" />
        </span>
        <span className="agent-card-main">
          <span className="agent-card-title">{t("message.summaryCardTitle")}</span>
          <span className="agent-card-status">{statusLabel}</span>
        </span>
        <span className="agent-card-hint">
          {canOpen
            ? t("message.agentCardOpen")
            : open
              ? t("message.summaryCardCollapse")
              : t("message.summaryCardOpen")}
        </span>
      </button>
      {open && !canOpen && text ? (
        <div className="summary-card-detail">{text}</div>
      ) : null}
    </div>
  );
}
