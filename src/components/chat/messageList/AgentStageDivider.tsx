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
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          aria-hidden
        >
          <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
        </svg>
        <span className="agent-stage-name">{label}</span>
        <span className="agent-stage-tag">{t("agentFlow.stageTag")}</span>
      </span>
      <span className="agent-stage-line" />
    </div>
  );
}
