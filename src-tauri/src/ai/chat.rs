use std::sync::Arc;

use crate::ai::parameters::GenerationParameters;
use crate::ai::tokens::TokenUsage;

pub type TextDeltaCallback = Arc<dyn Fn(String) + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct ImageResult {
    pub bytes: Vec<u8>,
    pub mime: String,
}

#[derive(Debug, Clone, Default)]
pub struct GenerateResponse {
    pub images: Vec<ImageResult>,
    pub text: Option<String>,
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
    #[allow(dead_code)]
    pub is_error: bool,
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
}
