/**
 * Shared settings modal for project-level and session-level configuration.
 * Layout: left navigation rail + right content panel (mirrors the global
 * settings page), so both "项目设置" and "会话设置" share one consistent UI.
 */
import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { ModelParamSettings } from "../../types";
import { EMPTY_MODEL_PARAMS } from "../settings/llm/modelServices";

type ConfigSection = "prompt" | "model";

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
  historyHint?: string;
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
  { id: "model", label: "模型参数", desc: "采样 · 上下文", icon: TuneIcon },
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
  const [section, setSection] = useState<ConfigSection>("prompt");
  const [paramsResetKey, setParamsResetKey] = useState(0);
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

  const historyTurns = parseTurns(historyTurnsDraft) ?? 0;

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

            {section === "model" && (
              <>
                <div className="config-modal-section-head">
                  <h4 className="config-modal-section-title">模型参数</h4>
                  <p className="config-modal-section-desc">{paramsHint}</p>
                </div>

                <div className="cfg-params" key={paramsResetKey}>
                  <SliderParamRow
                    label="模型温度"
                    hint="控制随机性；越高越发散，越低越确定。"
                    value={llmParamsDraft.temperature}
                    defaultValue={0.7}
                    min={0}
                    max={2}
                    step={0.1}
                    marks={[{ v: 0, label: "0" }, { v: 0.7, label: "0.7" }, { v: 2, label: "2" }]}
                    onChange={(next) =>
                      setLlmParamsDraft((cur) => ({ ...cur, temperature: next }))
                    }
                  />

                  <SliderParamRow
                    label="Top-P"
                    hint="核采样阈值；与温度二选一调节即可。"
                    value={llmParamsDraft.top_p}
                    defaultValue={0.9}
                    min={0}
                    max={1}
                    step={0.05}
                    marks={[{ v: 0, label: "0" }, { v: 0.5, label: "0.5" }, { v: 1, label: "1" }]}
                    onChange={(next) =>
                      setLlmParamsDraft((cur) => ({ ...cur, top_p: next }))
                    }
                  />

                  <ContextSliderRow
                    value={historyTurns}
                    hint={error ?? historyHint}
                    isError={!!error}
                    onChange={(next) => {
                      setHistoryTurnsDraft(String(next));
                      setError(null);
                    }}
                  />

                  <TokenParamRow
                    value={llmParamsDraft.max_tokens}
                    onChange={(next) =>
                      setLlmParamsDraft((cur) => ({ ...cur, max_tokens: next }))
                    }
                  />

                  <SliderParamRow
                    label="频率惩罚"
                    hint="降低重复用词的倾向（frequency_penalty）。"
                    value={llmParamsDraft.frequency_penalty}
                    defaultValue={0}
                    min={-2}
                    max={2}
                    step={0.1}
                    marks={[{ v: -2, label: "-2" }, { v: 0, label: "0" }, { v: 2, label: "2" }]}
                    onChange={(next) =>
                      setLlmParamsDraft((cur) => ({ ...cur, frequency_penalty: next }))
                    }
                  />

                  <SliderParamRow
                    label="存在惩罚"
                    hint="鼓励谈论新主题（presence_penalty）。"
                    value={llmParamsDraft.presence_penalty}
                    defaultValue={0}
                    min={-2}
                    max={2}
                    step={0.1}
                    marks={[{ v: -2, label: "-2" }, { v: 0, label: "0" }, { v: 2, label: "2" }]}
                    onChange={(next) =>
                      setLlmParamsDraft((cur) => ({ ...cur, presence_penalty: next }))
                    }
                  />

                </div>

                <div className="cfg-params-footer">
                  <button
                    type="button"
                    className="cfg-reset-btn"
                    onClick={() => {
                      setLlmParamsDraft({ ...EMPTY_MODEL_PARAMS });
                      setParamsResetKey((k) => k + 1);
                    }}
                  >
                    <ResetIcon />
                    <span>重置</span>
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ─── Parameter rows（图片同款：开关 + 滑块 + 数值）────────────────────────────

function formatNum(value: number): string {
  return Number.isInteger(value) ? String(value) : String(Number(value.toFixed(2)));
}

function ParamSwitch({
  checked,
  onChange,
  title,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  title?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={title}
      title={title}
      className={`cfg-switch ${checked ? "cfg-switch--on" : ""}`}
      onClick={() => onChange(!checked)}
    >
      <span className="cfg-switch-thumb" />
    </button>
  );
}

interface SliderMark {
  v: number;
  label: string;
}

function trackStyle(pct: number): CSSProperties {
  const clamped = Math.max(0, Math.min(100, pct));
  return {
    ["--cfg-fill" as string]: `linear-gradient(to right, var(--ink) ${clamped}%, var(--line-strong) ${clamped}%)`,
  };
}

function SliderParamRow({
  label,
  hint,
  value,
  defaultValue,
  min,
  max,
  step,
  marks,
  onChange,
}: {
  label: string;
  hint?: string;
  value: number | null;
  defaultValue: number;
  min: number;
  max: number;
  step: number;
  marks: SliderMark[];
  onChange: (next: number | null) => void;
}) {
  const enabled = value !== null && value !== undefined;
  const current = enabled ? (value as number) : defaultValue;
  const pct = ((current - min) / (max - min)) * 100;

  return (
    <div className="cfg-param">
      <div className="cfg-param-head">
        <span className="cfg-param-label" title={hint}>
          {label}
        </span>
        <div className="cfg-param-trailing">
          {enabled && <span className="cfg-param-value">{formatNum(current)}</span>}
          <ParamSwitch
            checked={enabled}
            title={label}
            onChange={(on) => onChange(on ? defaultValue : null)}
          />
        </div>
      </div>
      {enabled && (
        <div className="cfg-slider-wrap">
          <input
            type="range"
            className="cfg-slider"
            min={min}
            max={max}
            step={step}
            value={current}
            style={trackStyle(pct)}
            onChange={(e) => onChange(Number(e.target.value))}
          />
          <div className="cfg-slider-marks">
            {marks.map((m) => (
              <span
                key={m.v}
                style={{ left: `${((m.v - min) / (max - min)) * 100}%` }}
              >
                {m.label}
              </span>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

function ContextSliderRow({
  value,
  hint,
  isError,
  onChange,
}: {
  value: number;
  hint?: string;
  isError?: boolean;
  onChange: (next: number) => void;
}) {
  const max = 100;
  const display = Math.min(value, max);
  const pct = (display / max) * 100;
  const marks: SliderMark[] = [
    { v: 0, label: "0" },
    { v: 25, label: "25" },
    { v: 50, label: "50" },
    { v: 75, label: "75" },
    { v: 100, label: "不限" },
  ];

  return (
    <div className="cfg-param">
      <div className="cfg-param-head">
        <span className="cfg-param-label">上下文数</span>
        <span className="cfg-param-value">{value >= max ? "不限" : value}</span>
      </div>
      <div className="cfg-slider-wrap">
        <input
          type="range"
          className="cfg-slider"
          min={0}
          max={max}
          step={1}
          value={display}
          style={trackStyle(pct)}
          onChange={(e) => onChange(Number(e.target.value))}
        />
        <div className="cfg-slider-marks">
          {marks.map((m) => (
            <span key={m.v} style={{ left: `${(m.v / max) * 100}%` }}>
              {m.label}
            </span>
          ))}
        </div>
      </div>
      {hint && <div className={`hint ${isError ? "is-error" : ""}`}>{hint}</div>}
    </div>
  );
}

function TokenParamRow({
  value,
  onChange,
}: {
  value: number | null;
  onChange: (next: number | null) => void;
}) {
  const enabled = value !== null && value !== undefined;
  const [draft, setDraft] = useState(value == null ? "" : String(value));

  const commit = (raw: string) => {
    setDraft(raw);
    const trimmed = raw.trim();
    if (trimmed === "") return; // 留空时保持开启，等待输入
    const parsed = Number.parseInt(trimmed, 10);
    if (Number.isFinite(parsed) && parsed >= 1) onChange(parsed);
  };

  return (
    <div className="cfg-param">
      <div className="cfg-param-head">
        <span className="cfg-param-label">最大 Token 数</span>
        <ParamSwitch
          checked={enabled}
          title="最大 Token 数"
          onChange={(on) => {
            if (on) {
              const fallback = draft.trim() || "2048";
              setDraft(fallback);
              onChange(Number.parseInt(fallback, 10) || 2048);
            } else {
              onChange(null);
            }
          }}
        />
      </div>
      {enabled && (
        <input
          type="number"
          className="field-input field-input--mono cfg-number"
          inputMode="numeric"
          min={1}
          step={1}
          value={draft}
          placeholder="2048"
          onChange={(e) => commit(e.target.value)}
        />
      )}
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
function ResetIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <path d="M3 12a9 9 0 1 0 2.6-6.4L3 8" />
      <path d="M3 3v5h5" />
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
