//! User-editable rewrite / expand prompts and sampling parameters.

use serde::{Deserialize, Serialize};

pub const DEFAULT_CHUNK_CHARS: i64 = 700;
pub const DEFAULT_EXPAND_MAX_TOKENS: i64 = 1600;
pub const DEFAULT_TEMPERATURE: f64 = 0.9;
pub const DEFAULT_TOP_P: f64 = 0.95;
pub const DEFAULT_TOP_K: i64 = 50;
pub const DEFAULT_REPEAT_PENALTY: f64 = 1.05;
pub const DEFAULT_JEV_DEVICE: &str = "cuda:0";

/// rewrite3 task instruction (the model's `INSTRUCTION.txt`). Used instead of
/// the rewrite instruction when jevnovel scoring is on. The `【】` after each
/// sentence carries the jevnovel score, `【NA】` means no score.
pub const DEFAULT_JEV_INSTRUCTION: &str = "\
你是一位小说编辑，正在帮作者改文。下面这段小说片段是AIGC生成的劣化稿，每句末尾的【】里是jevnovel判定的“烂完了”概率（0到1，越高代表这句越烂），【NA】表示该句没有打分。请先像真人编辑一样，从前往后逐段分析哪些地方不好、为什么、打算怎么改，分析要自然，不要分点、不要markdown；分析完之后另起一段，直接输出改写后的完整正文。

本次任务是“缩短”档：改写后正文的字数不要超过输入的劣化稿（按去掉【】标记后的字数算，约为原文的 0.4-1.0 倍，中位约 0.7 倍，理想情况明显短于原文）。不要为了凑字数而补充描写，把啰嗦、重复、冗余的句子压掉，只保留情节、人物和必要的细节；不得新增或改变情节，不得出现原文推断不出的设定、人物或事件。

保持情节走向、人物设定和主要事件不变，无论原文好坏都要重新组织语言，禁止连续照抄原文超过15字。分析时不要提参考稿、人类原作或八股库，就当没有这些东西。

原文：";

pub const DEFAULT_REWRITE_INSTRUCTION: &str = "\
你是一位小说编辑，正在帮作者改文。请把下面这段小说片段改写成自然、有作者个人风格的中文，保持情节走向、人物设定和主要事件不变。无论原文是否通顺、AI味是轻是重，都要重新组织语言，禁止连续照抄原文超过15字；原文越好，越要用同义改写而不是照搬。

原文：";

pub const DEFAULT_EXPAND_INSTRUCTION: &str = "\
你是一位小说编辑，正在帮作者改文。请把下面这段经过删减的小说片段扩写成自然、有作者个人风格的中文，目标长度约为现在的 1.6 倍，属于中等幅度扩写。保持情节走向、人物设定和对话内容不变；补回被删掉的环境、动作、心理、神态等细节，同时允许重新组织、改写原有的句子和描写，把简略处展开、把生硬的句子重塑得更舒展自然，不要只在原句后面做加法；不得新增或改变情节；禁止连续照抄原文超过15字，直接输出扩写后的正文。

草稿：";

fn default_true() -> bool {
    true
}

fn default_instruction_jev() -> String {
    DEFAULT_JEV_INSTRUCTION.to_string()
}

fn default_jev_device() -> String {
    DEFAULT_JEV_DEVICE.to_string()
}

fn default_instruction_rewrite() -> String {
    DEFAULT_REWRITE_INSTRUCTION.to_string()
}

fn default_instruction_expand() -> String {
    DEFAULT_EXPAND_INSTRUCTION.to_string()
}

fn default_temperature() -> f64 {
    DEFAULT_TEMPERATURE
}

fn default_top_p() -> f64 {
    DEFAULT_TOP_P
}

fn default_top_k() -> i64 {
    DEFAULT_TOP_K
}

fn default_expand_max_tokens() -> i64 {
    DEFAULT_EXPAND_MAX_TOKENS
}

fn default_repeat_penalty() -> f64 {
    DEFAULT_REPEAT_PENALTY
}

fn default_chunk_chars() -> i64 {
    DEFAULT_CHUNK_CHARS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewriteSettings {
    /// When false, the rewrite request sends only the passage, without the instruction.
    #[serde(default = "default_true")]
    pub rewrite_prompt_enabled: bool,
    /// When false, the expand request sends only the passage, without the instruction.
    #[serde(default = "default_true")]
    pub expand_prompt_enabled: bool,
    #[serde(default = "default_instruction_rewrite")]
    pub rewrite_instruction: String,
    #[serde(default = "default_instruction_expand")]
    pub expand_instruction: String,
    #[serde(default = "default_temperature")]
    pub temperature: f64,
    #[serde(default = "default_top_p")]
    pub top_p: f64,
    #[serde(default = "default_top_k")]
    pub top_k: i64,
    #[serde(default = "default_expand_max_tokens")]
    pub expand_max_tokens: i64,
    #[serde(default = "default_repeat_penalty")]
    pub repeat_penalty: f64,
    #[serde(default = "default_chunk_chars")]
    pub chunk_chars: i64,
    /// When true, long text is split locally before each request.
    #[serde(default = "default_true")]
    pub auto_chunk: bool,
    /// When true, every sentence is scored by jevnovel and the score is
    /// appended as `【0.04】` before the text goes to the rewrite model.
    #[serde(default)]
    pub jev_enabled: bool,
    /// Python interpreter that can run jevnovel's `infer.py`.
    #[serde(default)]
    pub jev_python: String,
    /// Directory of the jevnovel weights, which contains `infer.py`.
    #[serde(default)]
    pub jev_dir: String,
    #[serde(default = "default_jev_device")]
    pub jev_device: String,
    /// Instruction sent with the scored text (rewrite stage only).
    #[serde(default = "default_instruction_jev")]
    pub jev_instruction: String,
}

impl Default for RewriteSettings {
    fn default() -> Self {
        Self {
            rewrite_prompt_enabled: true,
            expand_prompt_enabled: true,
            rewrite_instruction: default_instruction_rewrite(),
            expand_instruction: default_instruction_expand(),
            temperature: DEFAULT_TEMPERATURE,
            top_p: DEFAULT_TOP_P,
            top_k: DEFAULT_TOP_K,
            expand_max_tokens: DEFAULT_EXPAND_MAX_TOKENS,
            repeat_penalty: DEFAULT_REPEAT_PENALTY,
            chunk_chars: DEFAULT_CHUNK_CHARS,
            auto_chunk: true,
            jev_enabled: false,
            jev_python: String::new(),
            jev_dir: String::new(),
            jev_device: default_jev_device(),
            jev_instruction: default_instruction_jev(),
        }
    }
}

pub fn normalize_settings(mut cfg: RewriteSettings) -> RewriteSettings {
    cfg.rewrite_instruction = filled_or(&cfg.rewrite_instruction, DEFAULT_REWRITE_INSTRUCTION);
    cfg.expand_instruction = filled_or(&cfg.expand_instruction, DEFAULT_EXPAND_INSTRUCTION);
    cfg.jev_instruction = filled_or(&cfg.jev_instruction, DEFAULT_JEV_INSTRUCTION);
    cfg.jev_python = cfg.jev_python.trim().to_string();
    cfg.jev_dir = cfg.jev_dir.trim().to_string();
    cfg.jev_device = filled_or(&cfg.jev_device, DEFAULT_JEV_DEVICE);
    cfg.temperature = cfg.temperature.clamp(0.0, 2.0);
    cfg.top_p = cfg.top_p.clamp(0.0, 1.0);
    cfg.top_k = cfg.top_k.clamp(1, 200);
    cfg.expand_max_tokens = cfg.expand_max_tokens.clamp(64, 8192);
    cfg.repeat_penalty = cfg.repeat_penalty.clamp(1.0, 2.0);
    // 切分长度不设上限、不设下限（仅保证至少 1，避免除零 / 死循环）。
    cfg.chunk_chars = cfg.chunk_chars.max(1);
    cfg
}

fn filled_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}
