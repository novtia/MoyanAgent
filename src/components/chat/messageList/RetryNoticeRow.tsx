import { useTranslation } from "react-i18next";

export interface RetryNoticeProps {
  attempt: number;
  maxRetries: number;
  delayMs?: number;
  reason?: string;
  active: boolean;
}

export function RetryNoticeRow({
  attempt,
  maxRetries,
  delayMs,
  reason,
  active,
}: RetryNoticeProps) {
  const { t } = useTranslation();
  const delaySecs =
    delayMs !== undefined && delayMs > 0
      ? Math.max(1, Math.round(delayMs / 1000))
      : null;

  const label = active
    ? delaySecs !== null
      ? t("message.retryWaitingWithDelay", {
          attempt,
          max: maxRetries,
          seconds: delaySecs,
        })
      : t("message.retryWaiting", { attempt, max: maxRetries })
    : t("message.retryDone", { attempt, max: maxRetries });

  return (
    <div
      className={`retry-notice-row ${active ? "is-active" : ""}`}
      role="status"
      title={reason || label}
    >
      <svg
        className="retry-notice-icon"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden
      >
        <path d="M21 12a9 9 0 0 0-9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" />
        <path d="M3 3v5h5" />
        <path d="M3 12a9 9 0 0 0 9 9 9.75 9.75 0 0 0 6.74-2.74L21 16" />
        <path d="M16 16h5v5" />
      </svg>
      <span className="retry-notice-text">{label}</span>
      <span className="retry-notice-badge">
        {attempt}/{maxRetries}
      </span>
    </div>
  );
}
