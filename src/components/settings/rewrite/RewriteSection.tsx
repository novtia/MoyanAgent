import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useSettings } from "../../../store/settings";
import type { RewriteSettings } from "../../../types";

const SAVE_DEBOUNCE_MS = 500;

export const DEFAULT_REWRITE_SETTINGS: RewriteSettings = {
  rewrite_prompt_enabled: true,
  expand_prompt_enabled: true,
  rewrite_instruction:
    "你是一位小说编辑，正在帮作者改文。请把下面这段小说片段改写成自然、有作者个人风格的中文，保持情节走向、人物设定和主要事件不变。无论原文是否通顺、AI味是轻是重，都要重新组织语言，禁止连续照抄原文超过15字；原文越好，越要用同义改写而不是照搬。\n\n原文：",
  expand_instruction:
    "你是一位小说编辑，正在帮作者改文。请把下面这段经过删减的小说片段扩写成自然、有作者个人风格的中文，目标长度约为现在的 1.6 倍，属于中等幅度扩写。保持情节走向、人物设定和对话内容不变；补回被删掉的环境、动作、心理、神态等细节，同时允许重新组织、改写原有的句子和描写，把简略处展开、把生硬的句子重塑得更舒展自然，不要只在原句后面做加法；不得新增或改变情节；禁止连续照抄原文超过15字，直接输出扩写后的正文。\n\n草稿：",
  temperature: 0.9,
  top_p: 0.95,
  top_k: 50,
  expand_max_tokens: 1600,
  repeat_penalty: 1.05,
  chunk_chars: 700,
  auto_chunk: true,
  jev_enabled: false,
  jev_python: "",
  jev_dir: "",
  jev_device: "cuda:0",
  jev_instruction:
    "你是一位小说编辑，正在帮作者改文。下面这段小说片段是AIGC生成的劣化稿，每句末尾的【】里是jevnovel判定的“烂完了”概率（0到1，越高代表这句越烂），【NA】表示该句没有打分。请先像真人编辑一样，从前往后逐段分析哪些地方不好、为什么、打算怎么改，分析要自然，不要分点、不要markdown；分析完之后另起一段，直接输出改写后的完整正文。\n\n本次任务是“缩短”档：改写后正文的字数不要超过输入的劣化稿（按去掉【】标记后的字数算，约为原文的 0.4-1.0 倍，中位约 0.7 倍，理想情况明显短于原文）。不要为了凑字数而补充描写，把啰嗦、重复、冗余的句子压掉，只保留情节、人物和必要的细节；不得新增或改变情节，不得出现原文推断不出的设定、人物或事件。\n\n保持情节走向、人物设定和主要事件不变，无论原文好坏都要重新组织语言，禁止连续照抄原文超过15字。分析时不要提参考稿、人类原作或八股库，就当没有这些东西。\n\n原文：",
};

function mergeRewrite(value: RewriteSettings | undefined): RewriteSettings {
  return { ...DEFAULT_REWRITE_SETTINGS, ...value };
}

export function RewriteSection() {
  const { t } = useTranslation();
  const settings = useSettings((s) => s.settings);
  const update = useSettings((s) => s.update);
  const current = mergeRewrite(settings?.rewrite);
  const [rewriteInstruction, setRewriteInstruction] = useState(current.rewrite_instruction);
  const [expandInstruction, setExpandInstruction] = useState(current.expand_instruction);
  const [jevInstruction, setJevInstruction] = useState(current.jev_instruction);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latest = useRef(current);
  latest.current = {
    ...current,
    rewrite_instruction: rewriteInstruction,
    expand_instruction: expandInstruction,
    jev_instruction: jevInstruction,
  };

  useEffect(() => {
    setRewriteInstruction(current.rewrite_instruction);
  }, [current.rewrite_instruction]);

  useEffect(() => {
    setJevInstruction(current.jev_instruction);
  }, [current.jev_instruction]);

  useEffect(() => {
    setExpandInstruction(current.expand_instruction);
  }, [current.expand_instruction]);

  useEffect(() => {
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, []);

  const commit = (patch: Partial<RewriteSettings>) => {
    void update({ rewrite: { ...latest.current, ...patch } });
  };

  const scheduleInstruction = (patch: Partial<RewriteSettings>) => {
    if (timer.current) clearTimeout(timer.current);
    timer.current = setTimeout(() => {
      void update({ rewrite: { ...latest.current, ...patch } });
    }, SAVE_DEBOUNCE_MS);
  };

  return (
    <div className="settings-stack rewrite-settings">
      <div className="settings-card">
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">{t("settings.rewrite.enableRewrite")}</div>
            <div className="settings-row-desc">{t("settings.rewrite.enableRewriteDesc")}</div>
          </div>
          <div className="settings-row-control">
            <button
              type="button"
              role="switch"
              aria-checked={current.rewrite_prompt_enabled}
              className={`settings-toggle ${current.rewrite_prompt_enabled ? "settings-toggle--on" : ""}`}
              onClick={() => commit({ rewrite_prompt_enabled: !current.rewrite_prompt_enabled })}
            >
              <span className="settings-toggle-thumb" />
            </button>
          </div>
        </div>
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">{t("settings.rewrite.autoChunk")}</div>
            <div className="settings-row-desc">{t("settings.rewrite.autoChunkDesc")}</div>
          </div>
          <div className="settings-row-control">
            <button
              type="button"
              role="switch"
              aria-checked={current.auto_chunk}
              className={`settings-toggle ${current.auto_chunk ? "settings-toggle--on" : ""}`}
              onClick={() => commit({ auto_chunk: !current.auto_chunk })}
            >
              <span className="settings-toggle-thumb" />
            </button>
          </div>
        </div>
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">{t("settings.rewrite.enableExpand")}</div>
            <div className="settings-row-desc">{t("settings.rewrite.enableExpandDesc")}</div>
          </div>
          <div className="settings-row-control">
            <button
              type="button"
              role="switch"
              aria-checked={current.expand_prompt_enabled}
              className={`settings-toggle ${current.expand_prompt_enabled ? "settings-toggle--on" : ""}`}
              onClick={() => commit({ expand_prompt_enabled: !current.expand_prompt_enabled })}
            >
              <span className="settings-toggle-thumb" />
            </button>
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-row">
          <div className="settings-row-main">
            <div className="settings-row-title">{t("settings.rewrite.jevEnable")}</div>
            <div className="settings-row-desc">{t("settings.rewrite.jevEnableDesc")}</div>
          </div>
          <div className="settings-row-control">
            <button
              type="button"
              role="switch"
              aria-checked={current.jev_enabled}
              className={`settings-toggle ${current.jev_enabled ? "settings-toggle--on" : ""}`}
              onClick={() => commit({ jev_enabled: !current.jev_enabled })}
            >
              <span className="settings-toggle-thumb" />
            </button>
          </div>
        </div>
        {current.jev_enabled && (
          <div className="settings-block">
            <div className="rewrite-jev-fields">
              <TextField
                label={t("settings.rewrite.jevPython")}
                hint={t("settings.rewrite.jevPythonHint")}
                value={current.jev_python}
                placeholder="python"
                onCommit={(value) => commit({ jev_python: value })}
              />
              <TextField
                label={t("settings.rewrite.jevDir")}
                hint={t("settings.rewrite.jevDirHint")}
                value={current.jev_dir}
                onCommit={(value) => commit({ jev_dir: value })}
              />
              <TextField
                label={t("settings.rewrite.jevDevice")}
                value={current.jev_device}
                placeholder="cuda:0"
                onCommit={(value) => commit({ jev_device: value })}
              />
            </div>
          </div>
        )}
      </div>

      <div className="settings-card">
        <div className="settings-block">
          <div className="settings-block-head">
            <div className="settings-row-title">{t("settings.rewrite.paramsTitle")}</div>
            <div className="settings-row-desc">{t("settings.rewrite.paramsDesc")}</div>
          </div>
          <div className="rewrite-param-grid">
          <NumberField
            label={t("settings.rewrite.temperature")}
            value={current.temperature}
            step="0.05"
            onCommit={(value) => commit({ temperature: value })}
          />
          <NumberField
            label={t("settings.rewrite.topP")}
            value={current.top_p}
            step="0.05"
            onCommit={(value) => commit({ top_p: value })}
          />
          <NumberField
            label={t("settings.rewrite.topK")}
            value={current.top_k}
            step="1"
            onCommit={(value) => commit({ top_k: value })}
          />
          <NumberField
            label={t("settings.rewrite.repeatPenalty")}
            value={current.repeat_penalty}
            step="0.01"
            onCommit={(value) => commit({ repeat_penalty: value })}
          />
          <NumberField
            label={t("settings.rewrite.chunkChars")}
            value={current.chunk_chars}
            step="50"
            onCommit={(value) => commit({ chunk_chars: value })}
          />
          <NumberField
            label={t("settings.rewrite.expandMaxTokens")}
            value={current.expand_max_tokens}
            step="100"
            onCommit={(value) => commit({ expand_max_tokens: value })}
          />
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-row settings-row--prompt">
          <div className="settings-row-main">
            <div className="settings-row-title">{t("settings.rewrite.rewritePrompt")}</div>
            <div className="settings-row-desc">{t("settings.rewrite.rewritePromptDesc")}</div>
            <textarea
              className="settings-tool-desc-editor"
              value={rewriteInstruction}
              spellCheck={false}
              onChange={(e) => {
                const next = e.target.value;
                setRewriteInstruction(next);
                scheduleInstruction({ rewrite_instruction: next });
              }}
            />
            <button
              type="button"
              className="btn rewrite-reset"
              onClick={() => {
                const next = DEFAULT_REWRITE_SETTINGS.rewrite_instruction;
                setRewriteInstruction(next);
                commit({ rewrite_instruction: next });
              }}
            >
              {t("settings.rewrite.resetPrompt")}
            </button>
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-row settings-row--prompt">
          <div className="settings-row-main">
            <div className="settings-row-title">{t("settings.rewrite.jevPrompt")}</div>
            <div className="settings-row-desc">{t("settings.rewrite.jevPromptDesc")}</div>
            <textarea
              className="settings-tool-desc-editor"
              value={jevInstruction}
              spellCheck={false}
              onChange={(e) => {
                const next = e.target.value;
                setJevInstruction(next);
                scheduleInstruction({ jev_instruction: next });
              }}
            />
            <button
              type="button"
              className="btn rewrite-reset"
              onClick={() => {
                const next = DEFAULT_REWRITE_SETTINGS.jev_instruction;
                setJevInstruction(next);
                commit({ jev_instruction: next });
              }}
            >
              {t("settings.rewrite.resetPrompt")}
            </button>
          </div>
        </div>
      </div>

      <div className="settings-card">
        <div className="settings-row settings-row--prompt">
          <div className="settings-row-main">
            <div className="settings-row-title">{t("settings.rewrite.expandPrompt")}</div>
            <div className="settings-row-desc">{t("settings.rewrite.expandPromptDesc")}</div>
            <textarea
              className="settings-tool-desc-editor"
              value={expandInstruction}
              spellCheck={false}
              onChange={(e) => {
                const next = e.target.value;
                setExpandInstruction(next);
                scheduleInstruction({ expand_instruction: next });
              }}
            />
            <button
              type="button"
              className="btn rewrite-reset"
              onClick={() => {
                const next = DEFAULT_REWRITE_SETTINGS.expand_instruction;
                setExpandInstruction(next);
                commit({ expand_instruction: next });
              }}
            >
              {t("settings.rewrite.resetPrompt")}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function TextField({
  label,
  hint,
  value,
  placeholder,
  onCommit,
}: {
  label: string;
  hint?: string;
  value: string;
  placeholder?: string;
  onCommit: (value: string) => void;
}) {
  const [draft, setDraft] = useState(value);

  useEffect(() => {
    setDraft(value);
  }, [value]);

  return (
    <label className="rewrite-param-field">
      <span>{label}</span>
      <input
        type="text"
        value={draft}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          if (draft !== value) onCommit(draft);
        }}
      />
      {hint && <span>{hint}</span>}
    </label>
  );
}

function NumberField({
  label,
  value,
  step,
  onCommit,
}: {
  label: string;
  value: number;
  step: string;
  onCommit: (value: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));

  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  return (
    <label className="rewrite-param-field">
      <span>{label}</span>
      <input
        type="number"
        step={step}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => {
          const next = Number(draft);
          if (!Number.isFinite(next)) {
            setDraft(String(value));
            return;
          }
          onCommit(next);
        }}
      />
    </label>
  );
}
