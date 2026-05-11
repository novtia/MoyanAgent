//! Agent execution engine.
//!
//! Three pieces:
//!
//! - [`ProviderEngine`]: thin wrapper around
//!   [`crate::ai::providers::ProviderFactory`] for a single provider call.
//! - [`run_chat_request`]: agent-layer chat entry point used by the
//!   single-turn image-generation flow. Registers a [`Task`] in the
//!   [`TaskStore`] and reports completion / failure.
//! - [`ProviderQueryEngine`]: concrete implementation of
//!   [`QueryEngine`]. Runs the structurally-correct tool loop (drains
//!   attachments at turn boundaries, executes `tool_use` blocks through
//!   the [`ToolPool`], honors `max_turns` and abort signals).
//!
//! Today none of the providers surface `tool_use` blocks via
//! [`GenerateResponse`], so the loop terminates after the first turn.
//! When provider-level tool support lands, the
//! [`ProviderEngine::run_turn`] return type is the single seam that
//! needs to forward `tool_use` requests upward.

use std::sync::Arc;

use crate::ai::agent::core::attachment::{Attachment, AttachmentKind};
use crate::ai::agent::core::context::ToolUseContext;
use crate::ai::agent::core::permission::{AllowAllResolver, PermissionRequest, PermissionResolver};
use crate::ai::agent::exec::query::{QueryEngine, QueryFuture, QueryRequest, QueryResult};
use crate::ai::agent::core::task::{Task, TaskId, TaskState, TaskStore};
use crate::ai::agent::tools::{ToolInvocation, ToolPool, ToolResult};
use crate::ai::agent::types::{AgentId, MessageEvent, MessageId};
use crate::ai::chat::{ChatRequest, GenerateResponse, TextDeltaCallback};
use crate::ai::providers::ProviderFactory;
use crate::error::AppResult;

/// One model turn as observed by the engine.
///
/// `tool_uses` is the forward-compatible extension point: when providers
/// learn to surface tool_use blocks, this is the field they populate.
#[derive(Debug, Default, Clone)]
pub struct EngineTurn {
    pub response: GenerateResponse,
    pub tool_uses: Vec<ToolUseRequest>,
}

/// Provider-emitted request to invoke a tool.
#[derive(Debug, Clone)]
pub struct ToolUseRequest {
    pub id: MessageId,
    pub tool_name: String,
    pub input: serde_json::Value,
}

/// Single-turn provider engine. Cheap to clone (shared `Arc<ProviderFactory>`).
#[derive(Clone)]
pub struct ProviderEngine {
    factory: Arc<ProviderFactory>,
}

impl Default for ProviderEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ProviderEngine {
    pub fn new() -> Self {
        Self {
            factory: Arc::new(ProviderFactory::default()),
        }
    }

    pub fn with_factory(factory: Arc<ProviderFactory>) -> Self {
        Self { factory }
    }

    /// Backwards-compatible single-call helper. Most existing call sites
    /// (image generation) use this.
    pub async fn run(
        &self,
        request: ChatRequest,
        on_text_delta: Option<TextDeltaCallback>,
    ) -> AppResult<GenerateResponse> {
        if let Some(cb) = on_text_delta {
            self.factory.chat_stream(request, cb).await
        } else {
            self.factory.chat(request).await
        }
    }

    /// Tool-loop-friendly variant: returns the response paired with any
    /// `tool_use` blocks the provider surfaced. Image-generation
    /// providers leave this empty; text providers that decode
    /// `tool_calls` populate [`GenerateResponse::tool_calls`] which we
    /// normalise here into [`ToolUseRequest`]s.
    pub async fn run_turn(
        &self,
        request: ChatRequest,
        on_text_delta: Option<TextDeltaCallback>,
    ) -> AppResult<EngineTurn> {
        let response = self.run(request, on_text_delta).await?;
        let tool_uses = response
            .tool_calls
            .iter()
            .map(|tc| ToolUseRequest {
                id: MessageId(tc.id.clone()),
                tool_name: tc.name.clone(),
                input: tc.arguments.clone(),
            })
            .collect();
        Ok(EngineTurn {
            response,
            tool_uses,
        })
    }
}

/// Concrete [`QueryEngine`] backed by [`ProviderEngine`].
#[derive(Clone)]
pub struct ProviderQueryEngine {
    provider: Arc<ProviderEngine>,
    resolver: Arc<dyn PermissionResolver>,
    /// Default max turns when [`QueryRequest::max_turns`] is `None`.
    default_max_turns: u32,
}

impl Default for ProviderQueryEngine {
    fn default() -> Self {
        Self {
            provider: Arc::new(ProviderEngine::new()),
            resolver: Arc::new(AllowAllResolver),
            default_max_turns: 8,
        }
    }
}

impl ProviderQueryEngine {
    pub fn new(provider: Arc<ProviderEngine>, resolver: Arc<dyn PermissionResolver>) -> Self {
        Self {
            provider,
            resolver,
            default_max_turns: 8,
        }
    }

    pub fn with_max_turns(mut self, max_turns: u32) -> Self {
        self.default_max_turns = max_turns;
        self
    }
}

impl QueryEngine for ProviderQueryEngine {
    fn query<'a>(
        &'a self,
        request: QueryRequest,
        context: Arc<ToolUseContext>,
        tools: Arc<ToolPool>,
    ) -> QueryFuture<'a> {
        Box::pin(async move {
            let QueryRequest {
                mut chat,
                source: _,
                max_turns,
                initial_attachments,
                on_text_delta,
            } = request;

            // Push initial attachments (e.g. drained task-notifications)
            // into the chat history as hidden user-meta turns. This
            // mirrors `prependUserContext` / `getAttachmentMessages` in
            // the TS query loop.
            inject_attachments_into_history(&mut chat, &initial_attachments);

            // Populate the tool schema. The engine is the source of
            // truth for which tools the model may call — host code only
            // needs to register tools into the `ToolPool`. When tools
            // are present we force the buffered (non-streaming) path so
            // tool_calls come back in a single shot.
            let on_text_delta = if chat.tools.is_empty() && !tools_is_empty(&tools) {
                chat.tools = collect_tool_definitions(&tools);
                None
            } else {
                on_text_delta
            };

            let max_turns = max_turns.unwrap_or(self.default_max_turns);
            let mut events: Vec<MessageEvent> = Vec::new();
            let mut usage = crate::ai::tokens::TokenUsage::default();
            let mut tool_call_count: u32 = 0;
            let mut final_text: Option<String> = None;
            let mut final_images = Vec::new();

            for _turn in 0..max_turns {
                if context.abort.aborted() {
                    return Ok(QueryResult {
                        final_text,
                        events,
                        usage,
                        tool_call_count,
                        images: final_images,
                    });
                }

                let turn = self
                    .provider
                    .run_turn(chat.clone(), on_text_delta.clone())
                    .await?;
                let EngineTurn {
                    response,
                    tool_uses,
                } = turn;

                if let Some(text) = response.text.as_ref() {
                    events.push(MessageEvent::Assistant {
                        id: MessageId::new(),
                        text: text.clone(),
                    });
                    final_text = Some(text.clone());
                }
                usage = response.usage.clone();
                final_images = response.images.clone();

                if tool_uses.is_empty() {
                    // Model produced no tool_use blocks → loop terminates.
                    return Ok(QueryResult {
                        final_text,
                        events,
                        usage,
                        tool_call_count,
                        images: final_images,
                    });
                }

                // Execute each requested tool_use. Replies are stashed
                // on `chat.tool_results` so the provider serialiser can
                // emit them in its native shape (e.g. OpenAI
                // `role: "tool"` messages) on the next turn.
                chat.tool_results.clear();
                for req in tool_uses {
                    events.push(MessageEvent::ToolUse {
                        id: req.id.clone(),
                        tool: req.tool_name.clone(),
                        input: req.input.clone(),
                    });

                    let invocation = ToolInvocation {
                        id: req.id.clone(),
                        input: req.input.clone(),
                        context: context.as_ref(),
                    };
                    let perm = PermissionRequest {
                        agent_id: &context.agent_id,
                        tool_name: &req.tool_name,
                        input: &req.input,
                        mode: context.permission_mode,
                        is_async: matches!(
                            context.query_source,
                            crate::ai::agent::types::QuerySource::Forked
                        ),
                        is_coordinator_worker: false,
                    };

                    let result = tools
                        .execute(&req.tool_name, invocation, perm, self.resolver.as_ref())
                        .await
                        .unwrap_or_else(|e| ToolResult::error(e.to_string()));

                    chat.tool_results.push(crate::ai::chat::ToolResultMessage {
                        tool_call_id: req.id.0.clone(),
                        content: result.content.clone(),
                        is_error: result.is_error,
                    });

                    events.push(MessageEvent::ToolResult {
                        id: req.id.clone(),
                        tool: req.tool_name,
                        output: result.content,
                        is_error: result.is_error,
                    });
                    tool_call_count += 1;
                }

                // Drain any nested-memory triggers that the tool calls
                // recorded (e.g. via `FileReadTool`). Matching memory
                // files are converted to hidden user-meta history turns
                // for the next provider call. Honors the
                // `loaded_nested_memory_paths` dedup set on the context.
                if let Some(uc) = context.user_context.as_ref() {
                    let attachments =
                        crate::ai::agent::memory::nested::collect_nested_memory(&context, uc);
                    inject_attachments_into_history(&mut chat, &attachments);
                }
            }

            Ok(QueryResult {
                final_text,
                events,
                usage,
                tool_call_count,
                images: final_images,
            })
        })
    }
}

/// True iff the tool pool is empty. We can't check
/// [`ToolPool::all`] directly because the iterator borrows `tools` —
/// the loop body needs `tools` later, so we cache the answer here.
fn tools_is_empty(tools: &ToolPool) -> bool {
    tools.all().is_empty()
}

/// Map the active [`ToolPool`] into provider-agnostic
/// [`crate::ai::chat::ToolDefinition`] entries.
fn collect_tool_definitions(tools: &ToolPool) -> Vec<crate::ai::chat::ToolDefinition> {
    tools
        .all()
        .into_iter()
        .map(|t| {
            let s = t.spec();
            crate::ai::chat::ToolDefinition {
                name: s.name.clone(),
                description: s.description.clone(),
                schema: s.schema.clone(),
            }
        })
        .collect()
}

/// Prepend attachments as hidden user-meta history turns so the model
/// sees them on the very next request.
pub fn inject_attachments_into_history(chat: &mut ChatRequest, attachments: &[Attachment]) {
    if attachments.is_empty() {
        return;
    }
    let mut injected: Vec<crate::ai::chat::HistoryTurn> = Vec::with_capacity(attachments.len());
    for att in attachments {
        let body = crate::ai::agent::core::attachment::render(att);
        injected.push(crate::ai::chat::HistoryTurn {
            role: "user".into(),
            text: Some(body),
            images: Vec::new(),
        });
        // Bookkeeping: mark notification-shaped attachments rendered so
        // they don't get re-drained next time.
        if let AttachmentKind::TaskNotification(_) = &att.kind {
            // No-op today; placeholder for richer dedupe (e.g. write into
            // ToolUseContext::loaded_nested_memory_paths analogue).
        }
    }
    // Place attachments at the *front* of history — they're system-
    // reminder messages that should be visible before any prior turn.
    injected.extend(std::mem::take(&mut chat.history));
    chat.history = injected;
}

/// Outcome of [`run_chat_request`]: provider response + task tracking.
pub struct AgentChatOutcome {
    pub response: GenerateResponse,
    pub agent_id: AgentId,
    pub task_id: TaskId,
}

/// Agent-layer entry point for single-turn chat generation.
pub async fn run_chat_request(
    engine: &ProviderEngine,
    store: &TaskStore,
    agent_type: &str,
    prompt: String,
    request: ChatRequest,
    on_text_delta: Option<TextDeltaCallback>,
) -> AppResult<AgentChatOutcome> {
    let agent_id = AgentId::new();
    let task = Task::new_local(agent_id.clone(), agent_type, prompt);
    let task_id = store.register(task);
    store.set_state(&task_id, TaskState::Running);

    match engine.run(request, on_text_delta).await {
        Ok(response) => {
            store.complete(&task_id, response.text.clone(), response.usage.clone());
            Ok(AgentChatOutcome {
                response,
                agent_id,
                task_id,
            })
        }
        Err(e) => {
            store.fail(&task_id, e.to_string());
            Err(e)
        }
    }
}
