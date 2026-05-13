/**
 * Project-level parameter settings modal.
 * All sessions in the project inherit these settings instead of their own.
 */
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useProject } from "../../store/project";
import type { ModelParamSettings, Project } from "../../types";
import { EMPTY_MODEL_PARAMS } from "../settings/llm/modelServices";
import { NUMERIC_FIELDS } from "../settings/llm/modelParams";
import { NumericParamField } from "../settings/llm/NumericParamField";

interface ProjectConfigModalProps {
  project: Project;
  onClose: () => void;
}

export function ProjectConfigModal({ project, onClose }: ProjectConfigModalProps) {
  const { t } = useTranslation();
  const updateConfig = useProject((s) => s.updateConfig);

  const [systemPromptDraft, setSystemPromptDraft] = useState(project.system_prompt ?? "");
  const [historyTurnsDraft, setHistoryTurnsDraft] = useState(
    String(project.history_turns ?? 10),
  );
  const [llmParamsDraft, setLlmParamsDraft] = useState<ModelParamSettings>({
    ...EMPTY_MODEL_PARAMS,
    ...(project.llm_params ?? EMPTY_MODEL_PARAMS),
  });
  const [contextWindowDraft, setContextWindowDraft] = useState(
    project.context_window != null ? String(project.context_window) : "",
  );
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const save = async () => {
    const turns = Number.parseInt(historyTurnsDraft.trim(), 10);
    if (!Number.isFinite(turns) || turns < 0 || turns > 200) {
      setError("历史消息条数需为 0-200 的整数。");
      return;
    }
    const cwRaw = contextWindowDraft.trim();
    let contextWindow: number | null = null;
    if (cwRaw !== "") {
      const cw = Number.parseInt(cwRaw, 10);
      if (!Number.isFinite(cw) || cw < 1000) {
        setError("上下文窗口需为 ≥ 1000 的整数，留空则不限制。");
        return;
      }
      contextWindow = cw;
    }
    setSaving(true);
    try {
      await updateConfig(
        project.id,
        systemPromptDraft,
        turns,
        { ...EMPTY_MODEL_PARAMS, ...llmParamsDraft },
        contextWindow,
      );
      onClose();
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onMouseDown={onClose}
    >
      <div className="modal session-config-modal" onMouseDown={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h3>项目设置 · {project.name}</h3>
          <button type="button" className="close" onClick={onClose}>
            关闭
          </button>
        </div>
        <div className="modal-body">
          <div className="session-config-form">
            <div className="row">
              <div className="field-label" style={{ marginBottom: 4, color: "var(--ink-mute)", fontSize: 12 }}>
                以下参数应用于该项目下所有会话，会话自身的参数设置不再生效。
              </div>
            </div>

            <div className="row">
              <label className="field-label">系统提示词</label>
              <textarea
                className="settings-system-prompt field-input field-input--lg"
                rows={7}
                value={systemPromptDraft}
                spellCheck={false}
                placeholder="应用于项目所有会话；留空则不发送 system 提示词。"
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
                  setError(null);
                }}
              />
              <div className={`hint ${error?.includes("历史") ? "is-error" : ""}`}>
                {error?.includes("历史") ? error : "0 表示不携带历史；适用于项目所有会话。"}
              </div>
            </div>

            <div className="row">
              <label className="field-label">上下文窗口上限（tokens）</label>
              <input
                type="number"
                className="field-input field-input--mono"
                min={1000}
                step={1000}
                placeholder="留空不限制"
                value={contextWindowDraft}
                onChange={(e) => {
                  setContextWindowDraft(e.target.value);
                  setError(null);
                }}
              />
              <div className={`hint ${error?.includes("上下文") ? "is-error" : ""}`}>
                {error?.includes("上下文")
                  ? error
                  : "覆盖项目下所有会话的上下文窗口上限；留空则跟随模型目录值。"}
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
              <div className="hint">
                应用于项目所有会话的请求体；留空则不发送对应字段。
              </div>
            </div>
          </div>
        </div>
        <div className="modal-foot">
          <button type="button" className="btn" onClick={onClose}>
            取消
          </button>
          <button
            type="button"
            className="btn primary"
            onClick={save}
            disabled={saving}
          >
            {saving ? "保存中" : "保存"}
          </button>
        </div>
      </div>
    </div>
  );
}
