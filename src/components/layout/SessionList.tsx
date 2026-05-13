import { useSession } from "../../store/session";
import { SessionItem } from "./SessionItem";

interface SessionListProps {
  onOpenChat?: () => void;
  /** When true, only shows sessions not assigned to any project. */
  unassignedOnly?: boolean;
}

export function SessionList({ onOpenChat, unassignedOnly }: SessionListProps) {
  const allSessions = useSession((s) => s.sessions);
  const activeId = useSession((s) => s.activeId);
  const sessions = unassignedOnly ? allSessions.filter(s => !s.project_id) : allSessions;

  return (
    <div className="chat-list">
      {sessions.map((s) => (
        <SessionItem
          key={s.id}
          session={s}
          isActive={activeId === s.id}
          onOpenChat={onOpenChat}
        />
      ))}
    </div>
  );
}
