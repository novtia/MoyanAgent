import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { useSession } from "../store/session";

function timeAgo(ts: number, t: TFunction): string {
  const now = Date.now();
  const diff = Math.max(0, now - ts);
  const min = Math.floor(diff / 60000);
  if (min < 1) return t("sessions.timeNow");
  if (min < 60) return t("sessions.timeMinutes", { n: min });
  const h = Math.floor(min / 60);
  if (h < 24) return t("sessions.timeHours", { n: h });
  const d = Math.floor(h / 24);
  if (d < 7) return t("sessions.timeDays", { n: d });
  return t("sessions.timeWeeks", { n: Math.floor(d / 7) });
}

interface SessionListProps {
  onOpenChat?: () => void;
}

export function SessionList({ onOpenChat }: SessionListProps) {
  const { t } = useTranslation();
  const sessions = useSession((s) => s.sessions);
  const activeId = useSession((s) => s.activeId);
  const switchTo = useSession((s) => s.switchTo);
  const rename = useSession((s) => s.rename);
  const remove = useSession((s) => s.remove);

  const [editingId, setEditingId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [hoverId, setHoverId] = useState<string | null>(null);

  const submitRename = async () => {
    if (!editingId) return;
    const title = draftTitle.trim();
    if (title) await rename(editingId, title);
    setEditingId(null);
    setDraftTitle("");
  };

  const onDelete = async (id: string, title: string) => {
    if (!window.confirm(t("sessions.deleteConfirm", { title }))) return;
    await remove(id);
  };

  return (
    <div className="chat-list">
      {sessions.map((s) => {
        const isActive = activeId === s.id;
        const showActions = isActive || hoverId === s.id;
        return (
          <div
            key={s.id}
            className={`chat-item ${isActive ? "active" : ""}`}
            onMouseEnter={() => setHoverId(s.id)}
            onMouseLeave={() => setHoverId((id) => (id === s.id ? null : id))}
            onClick={() => {
              if (editingId !== s.id) {
                switchTo(s.id);
                onOpenChat?.();
              }
            }}
          >
            {editingId === s.id ? (
              <input
                className="chat-rename field-input field-input--compact"
                value={draftTitle}
                autoFocus
                onChange={(e) => setDraftTitle(e.target.value)}
                onBlur={submitRename}
                onKeyDown={(e) => {
                  if (e.key === "Enter") submitRename();
                  if (e.key === "Escape") {
                    setEditingId(null);
                    setDraftTitle("");
                  }
                }}
                onClick={(e) => e.stopPropagation()}
              />
            ) : (
              <>
                <span className="chat-title" title={s.title}>
                  {s.title}
                </span>
                {showActions ? (
                  <span
                    className="chat-actions"
                    onClick={(e) => e.stopPropagation()}
                  >
                    <button
                      type="button"
                      title={t("sessions.renameTitle")}
                      onClick={() => {
                        setEditingId(s.id);
                        setDraftTitle(s.title);
                      }}
                    >
                      <PencilIcon />
                    </button>
                    <button
                      type="button"
                      title={t("sessions.deleteTitle")}
                      onClick={() => onDelete(s.id, s.title)}
                    >
                      <TrashIcon />
                    </button>
                  </span>
                ) : (
                  <span className="chat-meta">{timeAgo(s.updated_at, t)}</span>
                )}
              </>
            )}
          </div>
        );
      })}
    </div>
  );
}

function PencilIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.121 2.121 0 0 1 3 3L7 19l-4 1 1-4Z" />
    </svg>
  );
}
function TrashIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
      <polyline points="3 6 5 6 21 6" />
      <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
    </svg>
  );
}
