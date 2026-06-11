/**
 * Reusable session row — handles rename, settings modal, delete,
 * and click-to-open. Used by both SessionList and project session lists.
 */
import { useState, type MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { openContextMenu } from "../context-menu";
import { useSession } from "../../store/session";
import { dialog } from "../ui";
import type { SessionSummary } from "../../types";
import { EMPTY_MODEL_PARAMS } from "../settings/llm/modelServices";
import { ScopeConfigModal } from "./ScopeConfigModal";

export function timeAgo(ts: number, t: TFunction): string {
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

interface SessionItemProps {
  session: SessionSummary;
  isActive: boolean;
  /** Extra CSS class names for the outer div. */
  className?: string;
  onOpenChat?: () => void;
  /** When set, this session belongs to a project; "会话设置" opens the project config instead. */
  projectId?: string;
  onOpenProjectConfig?: () => void;
}

export function SessionItem({
  session: s,
  isActive,
  className = "",
  onOpenChat,
  projectId,
  onOpenProjectConfig,
}: SessionItemProps) {
  const { t } = useTranslation();
  const switchTo = useSession((st) => st.switchTo);
  const rename = useSession((st) => st.rename);
  const remove = useSession((st) => st.remove);
  const updateConfig = useSession((st) => st.updateConfig);

  const [editingId, setEditingId] = useState<string | null>(null);
  const [draftTitle, setDraftTitle] = useState("");
  const [configTarget, setConfigTarget] = useState<SessionSummary | null>(null);

  const submitRename = async () => {
    if (!editingId) return;
    const title = draftTitle.trim();
    if (title) await rename(editingId, title);
    setEditingId(null);
    setDraftTitle("");
  };

  const openSessionMenu = (event: ReactMouseEvent, session: SessionSummary) => {
    openContextMenu(event, [
      // 普通会话才显示会话设置；项目会话从项目菜单统一管理
      ...(!projectId ? [{
        id: "session-settings",
        label: "会话设置",
        onSelect: () => setConfigTarget(session),
      }] : []),
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
        onSelect: async () => {
          const ok = await dialog.confirm(
            t("sessions.deleteConfirm", { title: session.title }),
            { type: "danger", confirmLabel: t("common.delete"), title: t("sessions.deleteTitle") },
          );
          if (ok) remove(session.id);
        },
      },
    ]);
  };

  return (
    <>
      <div
        className={`chat-item ${isActive ? "active" : ""} ${className}`}
        onContextMenu={(e) => openSessionMenu(e, s)}
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
            <span className="chat-title" title={s.title}>{s.title}</span>
            <span className="chat-meta">{timeAgo(s.updated_at, t)}</span>
          </>
        )}
      </div>

      {configTarget && (
        <ScopeConfigModal
          title="会话设置"
          subtitle={configTarget.title}
          scopeNote="以下设置仅作用于当前会话。"
          promptPlaceholder="仅作用于当前会话；留空则本会话不发送 system 提示词。"
          historyHint="0 表示不携带历史；该参数只影响当前会话。"
          paramsHint="仅作用于当前会话的请求体；留空则不在请求中发送对应字段。"
          initial={{
            systemPrompt: configTarget.system_prompt ?? "",
            historyTurns: configTarget.history_turns ?? 10,
            llmParams: {
              ...EMPTY_MODEL_PARAMS,
              ...(configTarget.llm_params ?? EMPTY_MODEL_PARAMS),
            },
          }}
          onSave={async (systemPrompt, historyTurns, llmParams) => {
            await updateConfig(configTarget.id, systemPrompt, historyTurns, llmParams);
          }}
          onClose={() => setConfigTarget(null)}
        />
      )}
    </>
  );
}
