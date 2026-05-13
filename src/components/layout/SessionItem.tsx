/**
 * Reusable session row — handles rename, settings modal, delete,
 * and click-to-open. Used by both SessionList and project session lists.
 */
import { useState, type MouseEvent as ReactMouseEvent } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { openContextMenu } from "../context-menu";
import { useSession } from "../../store/session";
import type { ModelParamSettings, SessionSummary } from "../../types";
import { EMPTY_MODEL_PARAMS } from "../settings/llm/modelServices";
import { NUMERIC_FIELDS } from "../settings/llm/modelParams";
import { NumericParamField } from "../settings/llm/NumericParamField";

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
  const [systemPromptDraft, setSystemPromptDraft] = useState("");
  const [historyTurnsDraft, setHistoryTurnsDraft] = useState("10");
  const [llmParamsDraft, setLlmParamsDraft] = useState<ModelParamSettings>(EMPTY_MODEL_PARAMS);
  const [configError, setConfigError] = useState<string | null>(null);
  const [savingConfig, setSavingConfig] = useState(false);

  const submitRename = async () => {
    if (!editingId) return;
    const title = draftTitle.trim();
    if (title) await rename(editingId, title);
    setEditingId(null);
    setDraftTitle("");
  };

  const openConfig = (session: SessionSummary) => {
    setConfigTarget(session);
    setSystemPromptDraft(session.system_prompt ?? "");
    setHistoryTurnsDraft(String(session.history_turns ?? 10));
    setLlmParamsDraft({ ...EMPTY_MODEL_PARAMS, ...(session.llm_params ?? EMPTY_MODEL_PARAMS) });
    setConfigError(null);
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
      await updateConfig(configTarget.id, systemPromptDraft, parsed, {
        ...EMPTY_MODEL_PARAMS,
        ...llmParamsDraft,
      });
      setConfigTarget(null);
    } finally {
      setSavingConfig(false);
    }
  };

  const openSessionMenu = (event: ReactMouseEvent, session: SessionSummary) => {
    openContextMenu(event, [
      // 普通会话才显示会话设置；项目会话从项目菜单统一管理
      ...(!projectId ? [{
        id: "session-settings",
        label: "会话设置",
        onSelect: () => openConfig(session),
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
        onSelect: () => {
          if (window.confirm(t("sessions.deleteConfirm", { title: session.title }))) {
            remove(session.id);
          }
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
        <div className="modal-backdrop" role="presentation" onMouseDown={() => setConfigTarget(null)}>
          <div className="modal session-config-modal" onMouseDown={(e) => e.stopPropagation()}>
            <div className="modal-head">
              <h3>会话设置</h3>
              <button type="button" className="close" onClick={() => setConfigTarget(null)}>
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

                <div className="row session-config-llm-params">
                  <div className="field-label">思考 / 推理</div>
                  <div className="session-config-thinking-row">
                    <label className="session-config-checkbox">
                      <input
                        type="checkbox"
                        checked={llmParamsDraft.thinking_enabled === true}
                        onChange={(e) =>
                          setLlmParamsDraft((cur) => ({
                            ...cur,
                            thinking_enabled: e.target.checked ? true : null,
                          }))
                        }
                      />
                      <span>开启（OpenAI: reasoning_effort；Claude: output_config.effort）</span>
                    </label>
                    <label className="field-label session-config-thinking-effort-label">强度</label>
                    <select
                      className="field-input field-input--mono"
                      value={llmParamsDraft.thinking_effort ?? ""}
                      onChange={(e) =>
                        setLlmParamsDraft((cur) => ({
                          ...cur,
                          thinking_effort: e.target.value.trim() ? e.target.value.trim() : null,
                        }))
                      }
                      disabled={llmParamsDraft.thinking_enabled !== true}
                    >
                      <option value="">默认 high</option>
                      <option value="low">low</option>
                      <option value="medium">medium</option>
                      <option value="high">high</option>
                      <option value="max">max</option>
                    </select>
                  </div>
                </div>

                <div className="row session-config-llm-params">
                  <div className="field-label">模型采样参数</div>
                  <div className="settings-params-grid">
                    {NUMERIC_FIELDS.map((field) => (
                      <NumericParamField
                        key={field.key}
                        def={field}
                        value={llmParamsDraft[field.key]}
                        onCommit={(next) =>
                          setLlmParamsDraft((cur) => ({ ...cur, [field.key]: next }))
                        }
                        invalidLabel={t("settings.llm.paramInvalid")}
                        label={t(field.labelKey)}
                        hint={t(field.hintKey)}
                      />
                    ))}
                  </div>
                  <div className="hint">仅作用于当前会话的请求体；留空则不在请求中发送对应字段。</div>
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
