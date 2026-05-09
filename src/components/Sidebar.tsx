import { useTranslation } from "react-i18next";
import { SessionList } from "./SessionList";
import { useSession } from "../store/session";

interface SidebarProps {
  onOpenSettings: () => void;
  onOpenChat: () => void;
  settingsActive: boolean;
}

export function Sidebar({ onOpenSettings, onOpenChat, settingsActive }: SidebarProps) {
  const { t } = useTranslation();
  const createNew = useSession((s) => s.createNew);
  const sessions = useSession((s) => s.sessions);
  const onNewChat = async () => {
    await createNew();
    onOpenChat();
  };

  return (
    <aside className="side">
      <div className="side-top">
        <nav className="side-nav">
          <button
            type="button"
            className="side-nav-item"
            onClick={onNewChat}
          >
            <NewChatIcon />
            <span>{t("sidebar.newChat")}</span>
          </button>
          <button type="button" className="side-nav-item" disabled>
            <SearchIcon />
            <span>{t("sidebar.search")}</span>
          </button>
          <button type="button" className="side-nav-item" disabled>
            <SkillsIcon />
            <span>{t("sidebar.skills")}</span>
          </button>
          <button type="button" className="side-nav-item" disabled>
            <PluginIcon />
            <span>{t("sidebar.plugins")}</span>
          </button>
          <button type="button" className="side-nav-item" disabled>
            <ClockIcon />
            <span>{t("sidebar.automations")}</span>
          </button>
          <button type="button" className="side-nav-item" disabled>
            <FolderIcon />
            <span>{t("sidebar.project")}</span>
          </button>
        </nav>

        <div className="side-section">
          <div className="side-section-title">{t("sidebar.chats")}</div>
          <SessionList onOpenChat={onOpenChat} />
          {sessions.length === 0 && (
            <div className="side-empty">{t("sidebar.empty")}</div>
          )}
        </div>
      </div>

      <div className="side-bottom">
        <button
          type="button"
          className={`side-nav-item ${settingsActive ? "active" : ""}`}
          onClick={onOpenSettings}
        >
          <GearIcon />
          <span>{t("sidebar.settings")}</span>
        </button>
      </div>
    </aside>
  );
}

function NewChatIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </svg>
  );
}
function SearchIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="11" cy="11" r="7" />
      <path d="m20 20-3.5-3.5" />
    </svg>
  );
}
function SkillsIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 2 3 7l9 5 9-5-9-5Z" />
      <path d="m3 12 9 5 9-5" />
      <path d="m3 17 9 5 9-5" />
    </svg>
  );
}
function PluginIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3" y="3" width="7" height="7" rx="1" />
      <rect x="14" y="3" width="7" height="7" rx="1" />
      <rect x="3" y="14" width="7" height="7" rx="1" />
      <rect x="14" y="14" width="7" height="7" rx="1" />
    </svg>
  );
}
function ClockIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="9" />
      <path d="M12 7v5l3 2" />
    </svg>
  );
}
function FolderIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2Z" />
    </svg>
  );
}
function GearIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09a1.65 1.65 0 0 0 1.51-1 1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33h0a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h0a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82v0a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
  );
}
