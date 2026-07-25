import { useTranslation } from "react-i18next";
import { useSession } from "../../store/session";

export function ChatSessionBreadcrumb() {
  const { t } = useTranslation();
  const active = useSession((s) => s.active);
  const switchTo = useSession((s) => s.switchTo);

  if (!active) return null;

  const title = active.session.title || t("chat.defaultTitle");
  const parentId = active.session.parent_session_id;
  const isTemp = !!active.session.is_temporary && !!parentId;

  if (!isTemp) {
    return (
      <span className="chat-topbar-title" title={title}>
        {title}
      </span>
    );
  }

  const parentTitle =
    active.parent_title?.trim() || t("chat.tempSessionFallback");
  const currentTitle = title || t("chat.tempSessionFallback");

  return (
    <nav className="chat-topbar-breadcrumb" aria-label={t("chat.breadcrumbNav")}>
      <button
        type="button"
        className="chat-crumb-parent"
        onClick={() => void switchTo(parentId)}
        title={parentTitle}
      >
        {parentTitle}
      </button>
      <span className="chat-crumb-sep" aria-hidden>
        /
      </span>
      <span className="chat-crumb-current" title={currentTitle}>
        {currentTitle}
      </span>
    </nav>
  );
}
