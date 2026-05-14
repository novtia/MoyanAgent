//! Main model loop ↔ tool execution coupling.
//!
//! This file owns the trait surface for what TS calls `query.ts`: it
//! streams model output, runs tool calls, drains queued attachments at
//! turn boundaries and tracks usage / progress.
//!
//! The concrete implementation lives in
//! [`crate::ai::agent::exec::engine::ProviderQueryEngine`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::ai::agent::core::attachment::Attachment;
use crate::ai::agent::core::context::ToolUseContext;
use crate::ai::agent::tools::ToolPool;
use crate::ai::agent::types::{MessageEvent, QuerySource, TokenUsage};
use crate::ai::chat::{ChatRequest, TextDeltaCallback};
use crate::error::AppResult;

/// Callback invoked the moment a `tool_use` / `tool_result` event is
/// recorded in the engine. Mirrors [`TextDeltaCallback`] but for the
/// structured tool stream — the host uses it to forward an inline
/// `gen://tool` event to the UI in real time, in the exact order the
/// engine produced the events.
pub type ToolEventCallback = Arc<dyn Fn(&MessageEvent) + Send + Sync + 'static>;

/// Inputs to one `query()` call.
pub struct QueryRequest {
    pub chat: ChatRequest,
    pub source: QuerySource,
    pub max_turns: Option<u32>,
    pub initial_attachments: Vec<Attachment>,
    /// Optional streaming callback. Forwarded to the underlying provider
    /// on each turn.
    pub on_text_delta: Option<TextDeltaCallback>,
    /// Optional callback fired alongside every `ToolUse` / `ToolResult`
    /// event pushed into the response stream. Used by the host to emit
    /// `gen://tool` Tauri events that the UI weaves into the assistant
    /// message blocks inline with streaming text/thinking deltas.
    pub on_tool_event: Option<ToolEventCallback>,
}

impl QueryRequest {
    pub fn new(chat: ChatRequest, source: QuerySource) -> Self {
        Self {
            chat,
            source,
            max_turns: None,
            initial_attachments: Vec::new(),
            on_text_delta: None,
            on_tool_event: None,
        }
    }
}

/// Aggregate result of a full multi-turn query loop.
#[derive(Debug, Clone, Default)]
pub struct QueryResult {
    pub final_text: Option<String>,
    /// Reasoning / extended-thinking text from the final turn, when the
    /// provider returns it separately from the visible assistant reply
    /// (e.g. Claude `thinking` blocks, OpenAI `reasoning` stream).
    pub thinking_content: Option<String>,
    pub events: Vec<MessageEvent>,
    pub usage: TokenUsage,
    pub tool_call_count: u32,
    /// Images emitted on the *final* turn (today this is the only place
    /// images come from since image-generation providers are single-turn).
    pub images: Vec<crate::ai::chat::ImageResult>,
}

/// Async return type used by [`QueryEngine`].
pub type QueryFuture<'a> = Pin<Box<dyn Future<Output = AppResult<QueryResult>> + Send + 'a>>;

/// The query engine drives the model ↔ tool loop. Implementations should:
///
/// 1. Build the API request via the provider stack.
/// 2. Stream `assistant` / `tool_use` events back to the caller.
/// 3. For each `tool_use`, call [`ToolPool::execute`] and re-feed the result.
/// 4. At turn boundaries, drain attachments from the notification queue.
/// 5. Stop when the model emits a turn without `tool_use`, or `max_turns`
///    is exhausted, or `context.abort.aborted()` is true.
pub trait QueryEngine: Send + Sync {
    fn query<'a>(
        &'a self,
        request: QueryRequest,
        context: Arc<ToolUseContext>,
        tools: Arc<ToolPool>,
    ) -> QueryFuture<'a>;
}
