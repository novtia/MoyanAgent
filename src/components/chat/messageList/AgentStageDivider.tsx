import { useTranslation } from "react-i18next";

export function AgentStageDivider({ label }: { label: string }) {
  const { t } = useTranslation();
  return (
    <div className="agent-stage-divider" role="separator" aria-label={label}>
      <span className="agent-stage-line" />
      <span className="agent-stage-chip">
        <svg
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          strokeWidth="1.8"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <circle cx="12" cy="12" r="3" />
          <path d="M12 2v4M12 18v4M2 12h4M18 12h4" />
        </svg>
        <span className="agent-stage-name">{label}</span>
        <span className="agent-stage-tag">{t("agentFlow.stageTag")}</span>
      </span>
      <span className="agent-stage-line" />
    </div>
  );
}
