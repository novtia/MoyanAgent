use std::sync::Arc;

use crate::ai::parameters::GenerationParameters;
use crate::ai::tokens::TokenUsage;

#[derive(Debug, Clone, Default)]
pub struct StreamDelta {
    pub text: Option<String>,
    pub thinking: Option<String>,
}

impl StreamDelta {
    pub fn text(text: String) -> Self {
        Self {
            text: Some(text),
            thinking: None,
        }
    }

    pub fn thinking(thinking: String) -> Self {
        Self {
            text: None,
            thinking: Some(thinking),
        }
    }
}

/// Emit one [`StreamDelta::thinking`] per Unicode scalar so the UI can animate
/// a typewriter effect (upstream often batches several characters per chunk).
pub fn emit_thinking_deltas(cb: &TextDeltaCallback, chunk: &str) {
    for ch in chunk.chars() {
        (cb)(StreamDelta::thinking(ch.to_string()));
    }
}

pub type TextDeltaCallback = Arc<dyn Fn(StreamDelta) + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct ImageResult {
    pub bytes: Vec<u8>,
    pub mime: String,
}

/// Drop duplicate raster outputs (`bytes` + `mime` identical), preserving first-seen order.
///
/// Gemini / OpenRouter multimodal streams often expose the same image twice: structured
/// `inline_data` (or equivalent) plus a `data:image/...` URL in markdown `content`; both get
/// collected into [`ImageResult`] and would otherwise create duplicate DB rows and UI plates.
pub fn dedupe_image_results(images: Vec<ImageResult>) -> Vec<ImageResult> {
    let mut out = Vec::with_capacity(images.len());
    for img in images {
        let dup = out
            .iter()
            .any(|e: &ImageResult| e.mime == img.mime && e.bytes == img.bytes);
        if !dup {
            out.push(img);
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct GenerateResponse {
    pub images: Vec<ImageResult>,
    pub text: Option<String>,
    /// Model reasoning / extended-thinking text when the provider returns
    /// it separately from the visible assistant reply (OpenAI `reasoning`,
    /// Responses API reasoning stream, Claude `thinking` blocks, ...).
    pub thinking_content: Option<String>,
    pub usage: TokenUsage,
    /// Tool-use requests the model emitted on this turn.
    ///
    /// Image-generation providers (`openai`, `gemini`, `grok`, `ark`)
    /// leave this empty. Text providers that support function/tool
    /// calling (`claude`, `openai-responses`) populate it so the agent
    /// query loop can dispatch into [`crate::ai::agent::tools::ToolPool`].
    pub tool_calls: Vec<ProviderToolCall>,
}

/// A model-emitted tool invocation request. Mirrors the OpenAI
/// `tool_calls[].function` / Anthropic `tool_use` shapes after
/// provider-side normalisation.
#[derive(Debug, Clone)]
pub struct ProviderToolCall {
    /// Provider-supplied tool-call id; opaque, just round-trip it.
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct AttachmentBytes {
    pub bytes: Vec<u8>,
    pub mime: String,
}

#[derive(Debug, Clone)]
pub struct HistoryTurn {
    pub role: String,
    pub text: Option<String>,
    pub images: Vec<AttachmentBytes>,
    /// Reasoning/thinking text from a prior assistant turn.
    /// Required by some providers (e.g. DeepSeek thinking mode) when
    /// replaying conversation history that originally contained it.
    pub thinking_content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub sdk: String,
    pub endpoint: String,
    pub api_key: String,
}

/// Tool description exposed to the model. Provider-agnostic — each
/// provider serialises this into its own native shape (OpenAI
/// `tools[].function`, Anthropic `tools[]`, ...).
#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub schema: serde_json::Value,
}

/// Tool result the host sends back on the next turn. Mirrors OpenAI's
/// `role: "tool"` message and Anthropic's `tool_result` content block.
#[derive(Debug, Clone)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub content: serde_json::Value,
    /// Consumed by providers that flag tool errors explicitly (e.g.
    /// Anthropic's `tool_result.is_error`). OpenAI ignores it — error
    /// state is implicit in the textual content.
    pub is_error: bool,
}

/// The immediately preceding assistant turn that emitted tool calls.
///
/// Providers that require the full call/response chain in the message
/// stream (OpenAI strict mode, Anthropic, Gemini) emit this between the
/// existing `history` and any `tool_results` so the conversation reads:
///
/// ```text
/// ... history ... → user prompt → assistant{text + tool_calls} → tool_results
/// ```
///
/// `None` ⇒ this is the first turn (or a turn after no tool_use), so no
/// extra message is required.
#[derive(Debug, Clone, Default)]
pub struct PendingAssistantTurn {
    pub text: Option<String>,
    /// DeepSeek / OpenAI-compat: prior-turn `reasoning_content` must be
    /// echoed on the assistant message when continuing after `tool_calls`.
    pub thinking_content: Option<String>,
    pub tool_calls: Vec<ProviderToolCall>,
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub provider: ProviderConfig,
    pub model: String,
    pub prompt: String,
    pub attachments: Vec<AttachmentBytes>,
    pub system_prompt: String,
    pub history: Vec<HistoryTurn>,
    pub parameters: GenerationParameters,
    /// Tools the model is allowed to call this turn. Empty ⇒ no tool
    /// schema is sent; provider behaviour falls back to plain chat.
    pub tools: Vec<ToolDefinition>,
    /// Replies to `tool_use` blocks emitted on the previous turn. Each
    /// entry is inserted into the provider message stream in the order
    /// listed; ordering matches the parent turn's tool_use ids.
    pub tool_results: Vec<ToolResultMessage>,
    /// The assistant turn that produced the `tool_results` above. Must
    /// be threaded back into the message stream — see
    /// [`PendingAssistantTurn`].
    pub pending_assistant_turn: Option<PendingAssistantTurn>,
}
