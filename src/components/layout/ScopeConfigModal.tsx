/**
 * Shared settings modal for project-level and session-level configuration.
 * Layout: left navigation rail + right content panel (mirrors the global
 * settings page), so both "项目设置" and "会话设置" share one consistent UI.
 */
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ModelParamSettings } from "../../types";
import { EMPTY_MODEL_PARAMS } from "../settings/llm/modelServices";
import { NUMERIC_FIELDS } from "../settings/llm/modelParams";
import { NumericParamField } from "../settings/llm/NumericParamField";

type ConfigSection = "prompt" | "context" | "model";

export interface ScopeConfigInitial {
  systemPrompt: string;
  historyTurns: number;
  llmParams: ModelParamSettings;
}

interface ScopeConfigModalProps {
  /** 弹窗主标题，如 "项目设置" / "会话设置" */
  title: string;
  /** 标题旁的对象名（项目名 / 会话名），可省略 */
  subtitle?: string;
  /** 作用范围说明，显示在底栏左侧 */
  scopeNote: string;
  promptPlaceholder: string;
  historyHint: string;
  paramsHint: string;
  initial: ScopeConfigInitial;
  onSave: (
    systemPrompt: string,
    historyTurns: number,
    llmParams: ModelParamSettings,
  ) => Promise<void>;
  onClose: () => void;
}

const SECTIONS: Array<{
  id: ConfigSection;
  label: string;
  desc: string;
  icon: () => JSX.Element;
}> = [
  { id: "prompt", label: "系统提示词", desc: "定义模型的角色与行为", icon: PromptIcon },
  { id: "context", label: "上下文", desc: "多轮会话历史携带策略", icon: HistoryIcon },
  { id: "model", label: "模型参数", desc: "思考推理与采样参数", icon: TuneIcon },
];

export function ScopeConfigModal({
  title,
  subtitle,
  scopeNote,
  promptPlaceholder,
  historyHint,
  paramsHint,
  initial,
  onSave,
  onClose,
}: ScopeConfigModalProps) {
  const { t } = useTranslation();

  const [section, setSection] = useState<ConfigSection>("prompt");
  const [systemPromptDraft, setSystemPromptDraft] = useState(initial.systemPrompt);
  const [historyTurnsDraft, setHistoryTurnsDraft] = useState(String(initial.historyTurns));
  const [llmParamsDraft, setLlmParamsDraft] = useState<ModelParamSettings>({
    ...EMPTY_MODEL_PARAMS,
    ...initial.llmParams,
  });
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [savedOnce, setSavedOnce] = useState(false);

  // 上一次已持久化内容的快照，避免重复保存
  const savedRef = useRef(
    JSON.stringify([
      initial.systemPrompt,
      initial.historyTurns,
      { ...EMPTY_MODEL_PARAMS, ...initial.llmParams },
    ]),
  );

  const parseTurns = (raw: string): number | null => {
    const turns = Number.parseInt(raw.trim(), 10);
    return Number.isFinite(turns) && turns >= 0 && turns <= 200 ? turns : null;
  };

  // 自动保存：草稿变化 400ms 后持久化
  useEffect(() => {
    const turns = parseTurns(historyTurnsDraft);
    if (turns === null) {
      setError("历史消息条数需为 0-200 的整数。");
      return;
    }
    setError(null);
    const params = { ...EMPTY_MODEL_PARAMS, ...llmParamsDraft };
    const snapshot = JSON.stringify([systemPromptDraft, turns, params]);
    if (snapshot === savedRef.current) return;
    const timer = setTimeout(async () => {
      setSaving(true);
      try {
        await onSave(systemPromptDraft, turns, params);
        savedRef.current = snapshot;
        setSavedOnce(true);
      } finally {
        setSaving(false);
      }
    }, 400);
    return () => clearTimeout(timer);
  }, [systemPromptDraft, historyTurnsDraft, llmParamsDraft, onSave]);

  // 关闭前冲刷未落盘的更改（防抖窗口内关闭不丢失）
  const handleClose = () => {
    const turns = parseTurns(historyTurnsDraft);
    if (turns !== null) {
      const params = { ...EMPTY_MODEL_PARAMS, ...llmParamsDraft };
      const snapshot = JSON.stringify([systemPromptDraft, turns, params]);
      if (snapshot !== savedRef.current) {
        savedRef.current = snapshot;
        void onSave(systemPromptDraft, turns, params);
      }
    }
    onClose();
  };

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") handleClose();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  });

  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={handleClose}>
      <div className="modal config-modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>
            {title}
            {subtitle && <span className="config-modal-subtitle">{subtitle}</span>}
          </h3>
          <div className="config-modal-head-trailing">
            <span className={`config-modal-autosave ${saving ? "is-saving" : ""}`}>
              {saving ? "保存中…" : savedOnce ? "已自动保存" : "更改将自动保存"}
            </span>
            <button type="button" className="close" onClick={handleClose}>
              关闭
            </button>
          </div>
        </div>

        <div className="modal-body config-modal-body">
          <nav className="config-modal-nav">
            {SECTIONS.map(({ id, label, desc, icon: Icon }) => (
              <button
                key={id}
                type="button"
                className={`config-modal-nav-item ${section === id ? "active" : ""}`}
                onClick={() => setSection(id)}
              >
                <span className="config-modal-nav-icon">
                  <Icon />
                </span>
                <span className="config-modal-nav-text">
                  <span className="config-modal-nav-label">{label}</span>
                  <span className="config-modal-nav-desc">{desc}</span>
                </span>
              </button>
            ))}
            <div className="config-modal-nav-note">{scopeNote}</div>
          </nav>

          <div className="config-modal-content" key={section}>
            {section === "prompt" && (
              <>
                <div className="config-modal-section-head">
                  <h4 className="config-modal-section-title">系统提示词</h4>
                  <p className="config-modal-section-desc">
                    作为 system 消息发送给模型，用于设定角色、语气与约束。
                  </p>
                </div>
                <textarea
                  className="field-input field-input--lg config-modal-prompt"
                  value={systemPromptDraft}
                  spellCheck={false}
                  placeholder={promptPlaceholder}
                  onChange={(e) => setSystemPromptDraft(e.target.value)}
                />
              </>
            )}

            {section === "context" && (
              <>
                <div className="config-modal-section-head">
                  <h4 className="config-modal-section-title">上下文</h4>
                  <p className="config-modal-section-desc">
                    控制每次请求携带多少条历史消息。
                  </p>
                </div>
                <div className="row">
                  <label className="field-label">多轮会话历史条数</label>
                  <input
                    type="number"
                    className="field-input field-input--mono config-modal-history-input"
                    min={0}
                    max={200}
                    step={1}
                    value={historyTurnsDraft}
                    onChange={(e) => {
                      setHistoryTurnsDraft(e.target.value);
                      setError(null);
                    }}
                  />
                  <div className={`hint ${error ? "is-error" : ""}`}>
                    {error ?? historyHint}
                  </div>
                </div>
              </>
            )}

            {section === "model" && (
              <>
                <div className="config-modal-section-head">
                  <h4 className="config-modal-section-title">模型参数</h4>
                  <p className="config-modal-section-desc">{paramsHint}</p>
                </div>

                <div className="config-modal-group">
                  <div className="config-modal-group-title">思考 / 推理</div>
                  <div className="config-thinking-row">
                    <label className="config-checkbox">
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
                    <label className="field-label config-thinking-effort-label">强度</label>
                    <select
                      className="field-input field-input--mono config-thinking-effort-select"
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

                <div className="config-modal-group">
                  <div className="config-modal-group-title">采样参数</div>
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
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Icons ────────────────────────────────────────────────────────────────────

function PromptIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2Z" />
      <path d="M8 9h8M8 13h5" />
    </svg>
  );
}
function HistoryIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 12a9 9 0 1 0 3-6.7L3 8" />
      <path d="M3 3v5h5" />
      <path d="M12 7v5l3 2" />
    </svg>
  );
}
function TuneIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3" />
      <path d="M1 14h6M9 8h6M17 16h6" />
    </svg>
  );
}
