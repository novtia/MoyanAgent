import { useState, type MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { openContextMenu } from "../context-menu";
import { useSession } from "../../store/session";
import type { SessionSummary } from "../../types";

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
  const updateConfig = useSession((s) => s.updateConfig);

  const [editingId, setEditingId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [configTarget, setConfigTarget] = useState<SessionSummary | null>(null);
  const [systemPromptDraft, setSystemPromptDraft] = useState("");
  const [historyTurnsDraft, setHistoryTurnsDraft] = useState("10");
  const [configError, setConfigError] = useState<string | null>(null);
  const [savingConfig, setSavingConfig] = useState(false);

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

  const openConfig = (session: SessionSummary) => {
    setConfigTarget(session);
    setSystemPromptDraft(session.system_prompt ?? "");
    setHistoryTurnsDraft(String(session.history_turns ?? 10));
    setConfigError(null);
  };

  const openSessionMenu = (event: ReactMouseEvent, session: SessionSummary) => {
    openContextMenu(event, [
      {
        id: "session-settings",
        label: "会话设置",
        onSelect: () => openConfig(session),
      },
      {
        id: "session-rename",
        label: t("sessions.renameTitle"),
        onSelect: () => {
          setEditingId(session.id);
          setDraftTitle(session.title);
        },
      },
      { type: "separator" },
      {
        id: "session-delete",
        label: t("sessions.deleteTitle"),
        danger: true,
        onSelect: () => onDelete(session.id, session.title),
      },
    ]);
  };

  const saveConfig = async () => {
    if (!configTarget) return;
    const parsed = Number.parseInt(historyTurnsDraft.trim(), 10);
    if (!Number.isFinite(parsed) || parsed < 0 || parsed > 200) {
      setConfigError("历史消息条数需为 0-200 的整数。");
      return;
    }
    setSavingConfig(true);
    try {
      await updateConfig(configTarget.id, systemPromptDraft, parsed);
      setConfigTarget(null);
    } finally {
      setSavingConfig(false);
    }
  };

  return (
    <>
      <div className="chat-list">
        {sessions.map((s) => {
          const isActive = activeId === s.id;
          return (
            <div
              key={s.id}
              className={`chat-item ${isActive ? "active" : ""}`}
              onContextMenu={(event) => openSessionMenu(event, s)}
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
                  onContextMenu={(e) => e.stopPropagation()}
                />
              ) : (
                <>
                  <span className="chat-title" title={s.title}>
                    {s.title}
                  </span>
                  <span className="chat-meta">{timeAgo(s.updated_at, t)}</span>
                </>
              )}
            </div>
          );
        })}
      </div>

      {configTarget && (
        <div className="modal-backdrop" role="presentation" onMouseDown={() => setConfigTarget(null)}>
          <div className="modal session-config-modal" onMouseDown={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>会话设置</h3>
              <button
                type="button"
                className="close"
                onClick={() => setConfigTarget(null)}
              >
                关闭
              </button>
            </div>
            <div className="modal-body">
              <div className="session-config-form">
                <div className="row">
                  <label className="field-label">系统提示词</label>
                  <textarea
                    className="settings-system-prompt field-input field-input--lg"
                    rows={7}
                    value={systemPromptDraft}
                    spellCheck={false}
                    placeholder="仅作用于当前会话；留空则本会话不发送 system 提示词。"
                    onChange={(e) => setSystemPromptDraft(e.target.value)}
                  />
                </div>
                <div className="row">
                  <label className="field-label">多轮会话历史条数</label>
                  <input
                    type="number"
                    className="field-input field-input--mono"
                    min={0}
                    max={200}
                    step={1}
                    value={historyTurnsDraft}
                    onChange={(e) => {
                      setHistoryTurnsDraft(e.target.value);
                      setConfigError(null);
                    }}
                  />
                  <div className={`hint ${configError ? "is-error" : ""}`}>
                    {configError ?? "0 表示不携带历史；该参数只影响当前会话。"}
                  </div>
                </div>
              </div>
            </div>
            <div className="modal-foot">
              <button type="button" className="btn" onClick={() => setConfigTarget(null)}>
                取消
              </button>
              <button
                type="button"
                className="btn primary"
                onClick={saveConfig}
                disabled={savingConfig}
              >
                {savingConfig ? "保存中" : "保存"}
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}
