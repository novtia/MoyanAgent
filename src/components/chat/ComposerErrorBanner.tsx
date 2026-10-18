import { useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";
import type { MessageAbs } from "../../types";
import { nowStamp } from "./messageList/utils";

/** Latest turn failed — show that error above the composer instead of in the transcript. */
export function trailingComposerError(messages: MessageAbs[]): MessageAbs | null {
  const last = messages[messages.length - 1];
  if (!last || last.role !== "error") return null;
  if (!last.text?.trim()) return null;
  return last;
}

let dismissedErrorId: string | null = null;
const dismissListeners = new Set<() => void>();

function subscribeDismiss(listener: () => void) {
  dismissListeners.add(listener);
  return () => dismissListeners.delete(listener);
}

function getDismissedErrorId() {
  return dismissedErrorId;
}

export function dismissComposerError(messageId: string) {
  if (dismissedErrorId === messageId) return;
  dismissedErrorId = messageId;
  dismissListeners.forEach((listener) => listener());
}

export function useComposerErrorDismissed(messageId: string | null) {
  const dismissedId = useSyncExternalStore(
    subscribeDismiss,
    getDismissedErrorId,
    getDismissedErrorId,
  );
  return messageId != null && dismissedId === messageId;
}

export function ComposerErrorBanner() {
  const { t } = useTranslation();
  const id = useSession((s) => trailingComposerError(s.active?.messages ?? [])?.id ?? null);
  const text = useSession((s) => trailingComposerError(s.active?.messages ?? [])?.text ?? null);
  const createdAt = useSession(
    (s) => trailingComposerError(s.active?.messages ?? [])?.created_at ?? null,
  );
  const dismissed = useComposerErrorDismissed(id);
  if (!id || !text || createdAt == null || dismissed) return null;

  return (
    <div className="composer-error" role="alert">
      <div className="composer-error-body">
        <span className="composer-error-stamp">
          {t("message.stampFailure", { time: nowStamp(createdAt) })}
        </span>
        <div className="composer-error-text">{text}</div>
      </div>
      <button
        type="button"
        className="composer-error-close"
        aria-label={t("common.close")}
        title={t("common.close")}
        onClick={() => dismissComposerError(id)}
      >
        ×
      </button>
    </div>
  );
}
