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
//!   the [`ToolPool`], honors abort signals).
//!
//! Today none of the providers surface `tool_use` blocks via
//! [`GenerateResponse`], so the loop terminates after the first turn.
//! When provider-level tool support lands, the
//! [`ProviderEngine::run_turn`] return type is the single seam that
//! needs to forward `tool_use` requests upward.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use crate::ai::agent::core::attachment::{Attachment, AttachmentKind};
use crate::ai::agent::core::context::ToolUseContext;
use crate::ai::agent::core::permission::{AllowAllResolver, PermissionRequest, PermissionResolver};
use crate::ai::agent::core::task::{Task, TaskId, TaskState, TaskStore};
use crate::ai::agent::exec::query::{
    QueryEngine, QueryFuture, QueryRequest, QueryResult, ToolEventCallback,
};
use crate::ai::agent::tools::{ToolInvocation, ToolPool, ToolResult};
use crate::ai::agent::types::{AgentId, MessageEvent, MessageId};
use crate::ai::chat::{
    emit_text_deltas, emit_thinking_deltas, ChatRequest, GenerateResponse, TextDeltaCallback,
};
use crate::ai::providers::ProviderFactory;
use crate::ai::token_log::{ApiCallLog, LogContext, ToolCallLog};
use crate::error::{AppError, AppResult};

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
    pub thought_signature: Option<String>,
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
                thought_signature: tc.thought_signature.clone(),
            })
            .collect();
        Ok(EngineTurn {
            response,
            tool_uses,
        })
    }
}

/// Tool-call rounds allowed per user turn when the agent definition does not
/// set `maxTurns`.
///
/// Generous enough for genuinely long multi-file work, but bounded: the loop
/// has no other exit while the model keeps emitting tool calls, and every round
/// re-sends the entire prompt.
const DEFAULT_MAX_TURNS: u32 = 60;

/// Concrete [`QueryEngine`] backed by [`ProviderEngine`].
#[derive(Clone)]
pub struct ProviderQueryEngine {
    provider: Arc<ProviderEngine>,
    resolver: Arc<dyn PermissionResolver>,
}

impl Default for ProviderQueryEngine {
    fn default() -> Self {
        Self {
            provider: Arc::new(ProviderEngine::new()),
            resolver: Arc::new(AllowAllResolver),
        }
    }
}

impl ProviderQueryEngine {
    pub fn new(provider: Arc<ProviderEngine>, resolver: Arc<dyn PermissionResolver>) -> Self {
        Self { provider, resolver }
    }
}

/// Clears this run's TodoList when the query future completes (ok, err, or cancel).
struct ClearTodoOnDrop {
    tools: Arc<ToolPool>,
    context: Arc<ToolUseContext>,
}

impl Drop for ClearTodoOnDrop {
    fn drop(&mut self) {
        self.tools.clear_todo_scope(self.context.as_ref());
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
                tool_anchor,
                on_text_delta,
                on_tool_event,
            } = request;

            // Push initial attachments (e.g. drained task-notifications)
            // into the chat history as hidden user-meta turns.
            inject_attachments_into_history(&mut chat, &initial_attachments);

            // Populate the tool schema. The engine is the source of truth
            // for which tools the model may call — host code only needs to
            // register tools into the `ToolPool`. Anchored agents get a
            // deliberately short catalog for the first request only; see
            // [`ToolAnchor`].
            let mut anchor: Option<ToolAnchor> = None;
            if chat.tools.is_empty() && !tools_is_empty(&tools) {
                // A request that continues a provider-side cache chain sends
                // no tool list at all — the chain head's list still governs —
                // so there is nothing to stage here, and releasing later would
                // break the chain for no change in what the model sees. Later
                // turns of a cached session are already inside the trajectory
                // the first one established, which is what anchoring buys.
                let resuming = continues_cache_chain(chat.previous_response_id.as_deref());
                let staging: &[String] = if resuming { &[] } else { &tool_anchor };
                let staged = ToolAnchor::new(collect_tool_definitions(&tools), staging);
                chat.tools = staged.initial();
                anchor = staged.is_staged().then_some(staged);
            }
            chat.forced_tools = crate::ai::agent::tools::forced_tool_names();

            let _todo_guard = ClearTodoOnDrop {
                tools: tools.clone(),
                context: context.clone(),
            };

            let mut events: Vec<MessageEvent> = Vec::new();
            let mut usage = crate::ai::tokens::TokenUsage::default();
            let mut tool_call_count: u32 = 0;
            let mut final_text: Option<String> = None;
            let mut final_thinking: Option<String> = None;
            let mut final_images = Vec::new();
            let mut final_videos = Vec::new();

            // When the model tries to stop with unfinished TodoList items,
            // keep looping and re-inject the live ✔/☐ snapshot at the END
            // of the next request. History nudges get buried once the
            // transcript is long — that's why the model used to forget
            // remaining items.
            const MAX_TODO_NUDGES: u32 = 32;
            let turn_limit = max_turns.unwrap_or(DEFAULT_MAX_TURNS).max(1);
            let mut turn_count: u32 = 0;
            let mut todo_nudges: u32 = 0;
            let mut premature_todo_stop = false;
            // Host-facing slot for a window read out of an upstream rejection.
            // Success paths never populate it after compaction was removed;
            // generation still recovers the value from the error report.
            let observed_context_window: Option<i64> = None;

            loop {
                if context.abort.aborted() {
                    return Err(AppError::Canceled);
                }

                chat.todo_snapshot =
                    tools.todo_prompt_snapshot(context.as_ref(), premature_todo_stop);
                premature_todo_stop = false;

                turn_count += 1;
                if turn_count > turn_limit {
                    return Ok(QueryResult {
                        final_text: Some(turn_limit_notice(turn_limit, final_text.as_deref())),
                        thinking_content: final_thinking,
                        events,
                        usage,
                        tool_call_count,
                        images: final_images,
                        videos: final_videos,
                        response_id: chat.previous_response_id.clone(),
                        observed_context_window,
                    });
                }

                // Stream every turn — including tool-call turns and the
                // final-answer turn. Provider streaming paths now
                // correctly accumulate `tool_calls` deltas, so there is
                // no longer any reason to fall back to a non-streaming
                // call once `tool_results` are pending. Live blocks the
                // host renders therefore reflect every thinking / text
                // delta in real time across the whole agent loop.
                //
                // The callback is wrapped in a tracker so we can detect
                // providers whose `chat_stream` impl is actually a
                // synchronous `chat()` fallback (the default trait impl
                // used by Claude / Gemini / Grok / Ark image SDK). In
                // that case the wrapped callback is never invoked
                // during the turn — we replay the response's
                // text/thinking once afterwards so the host's block
                // buffer still matches what the model produced.
                let (turn_delta, tracker) = match on_text_delta.as_ref() {
                    Some(cb) => {
                        let (wrapped, t) = wrap_tracking_callback(cb.clone());
                        (Some(wrapped), Some(t))
                    }
                    None => (None, None),
                };

                let turn_result = tokio::select! {
                    t = self.provider.run_turn(chat.clone(), turn_delta) => t,
                    _ = context.abort.wait_aborted() => {
                        return Err(AppError::Canceled);
                    }
                };
                let turn = match turn_result {
                    Ok(t) => t,
                    Err(e) => return Err(e),
                };
                let EngineTurn {
                    response,
                    tool_uses,
                } = turn;

                // Volcengine Session cache: relay response.id into the next
                // provider call as previous_response_id.
                if let Some(id) = response
                    .response_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    chat.previous_response_id = Some(id.to_string());
                }

                if let (Some(cb), Some(tracker)) = (on_text_delta.as_ref(), tracker.as_ref()) {
                    if tracker.thinking_chars.load(Ordering::Relaxed) == 0 {
                        if let Some(t) = response
                            .thinking_content
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            emit_thinking_deltas(cb, t);
                        }
                    }
                    if tracker.text_chars.load(Ordering::Relaxed) == 0 {
                        if let Some(t) = response.text.as_deref().filter(|s| !s.is_empty()) {
                            emit_text_deltas(cb, t);
                        }
                    }
                }

                if let Some(text) = response.text.as_ref() {
                    events.push(MessageEvent::Assistant {
                        id: MessageId::new(),
                        text: text.clone(),
                    });
                    final_text = Some(text.clone());
                }
                // Capture thinking/reasoning from the final (non-tool) turn.
                // Intermediate tool-call turns may also carry thinking content
                // but only the last assistant-facing turn is surfaced to the user.
                if let Some(ref t) = response.thinking_content {
                    if !t.trim().is_empty() {
                        final_thinking = Some(t.clone());
                    }
                }
                // Accumulate billing tokens across all tool-call rounds.
                accumulate_usage(&mut usage, &response.usage);
                if context.token_stats.is_some() || context.session_logger.is_some() {
                    let ctx = log_context(context.as_ref());
                    if let Some(stats) = context.token_stats.as_ref() {
                        stats.log_api_call(ApiCallLog {
                            ctx: ctx.clone(),
                            model: chat.model.clone(),
                            provider: chat.provider.id.clone(),
                            turn_index: turn_count,
                            usage: response.usage.clone(),
                        });
                    }
                    if let Some(logger) = context.session_logger.as_ref() {
                        logger.log_assistant_turn(
                            &ctx,
                            turn_count,
                            &chat.model,
                            &chat.provider.id,
                            &response,
                        );
                    }
                }
                final_images = response.images.clone();
                final_videos = response.videos.clone();

                if tool_uses.is_empty() {
                    if tools.incomplete_todo_nudge(context.as_ref()).is_some() {
                        if todo_nudges < MAX_TODO_NUDGES {
                            todo_nudges += 1;
                            premature_todo_stop = true;
                            // Discard the premature summary — work continues.
                            // The next iteration's todo_snapshot carries the
                            // "do not stop" rider at the end of the request.
                            final_text = None;
                            final_thinking = None;
                            continue;
                        }
                    }
                    // Model produced no tool_use blocks → loop terminates.
                    return Ok(QueryResult {
                        final_text,
                        thinking_content: final_thinking,
                        events,
                        usage,
                        tool_call_count,
                        images: final_images,
                        videos: final_videos,
                        response_id: chat.previous_response_id.clone(),
                        observed_context_window,
                    });
                }

                // Execute each requested tool_use. Replies are stashed
                // on `chat.tool_results` so the provider serialiser can
                // emit them in its native shape (e.g. OpenAI
                // `role: "tool"` messages) on the next turn.
                //
                // We also stash the assistant's *own* turn (the one
                // that emitted these tool calls) so providers that
                // require call/response symmetry (Anthropic, strict
                // OpenAI) can thread it back into the message stream.
                chat.tool_results.clear();
                chat.pending_assistant_turn = Some(crate::ai::chat::PendingAssistantTurn {
                    text: response.text.clone(),
                    thinking_content: response.thinking_content.clone(),
                    tool_calls: response.tool_calls.clone(),
                });
                for req in &tool_uses {
                    if context.abort.aborted() {
                        return Err(AppError::Canceled);
                    }
                    let use_event = MessageEvent::ToolUse {
                        id: req.id.clone(),
                        tool: req.tool_name.clone(),
                        input: req.input.clone(),
                        thought_signature: req.thought_signature.clone(),
                    };
                    fire_tool_event(&on_tool_event, &use_event);
                    events.push(use_event);

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

                    let result = tokio::select! {
                        r = tools.execute(
                            &req.tool_name,
                            invocation,
                            perm,
                            self.resolver.as_ref(),
                        ) => r.unwrap_or_else(|e| ToolResult::error(e.to_string())),
                        _ = context.abort.wait_aborted() => {
                            ToolResult::error("generation cancelled")
                        }
                    };

                    chat.tool_results.push(crate::ai::chat::ToolResultMessage {
                        tool_call_id: req.id.0.clone(),
                        content: result.content.clone(),
                        is_error: result.is_error,
                    });

                    if context.token_stats.is_some() || context.session_logger.is_some() {
                        let ctx = log_context(context.as_ref());
                        if let Some(stats) = context.token_stats.as_ref() {
                            stats.log_tool_call(ToolCallLog {
                                ctx: ctx.clone(),
                                tool_name: req.tool_name.clone(),
                                result: result.clone(),
                                input: req.input.clone(),
                            });
                        }
                        if let Some(logger) = context.session_logger.as_ref() {
                            logger.log_tool_call(&ctx, &req.tool_name, &req.input, &result);
                        }
                    }

                    let result_event = MessageEvent::ToolResult {
                        id: req.id.clone(),
                        tool: req.tool_name.clone(),
                        output: result.content,
                        is_error: result.is_error,
                    };
                    fire_tool_event(&on_tool_event, &result_event);
                    events.push(result_event);
                    tool_call_count += 1;
                }

                // Persist this round into tool_chain so the next loop
                // iteration sends the full in-turn tool history.
                commit_tool_round(&mut chat);

                // The model committed to the anchored trajectory by calling a
                // tool, so give it back the catalog it was actually filtered
                // for. Volcengine's Session cache pins tools to the chain head
                // and omits them from continuation requests, so the wider
                // catalog would never reach the model over an existing chain —
                // start a new one. That costs a single cache miss, once.
                if let Some(full) = anchor.as_mut().and_then(|a| a.release()) {
                    chat.tools = full;
                    chat.previous_response_id = None;
                }
            }
        })
    }
}

/// Helper: forward a structural tool event to the host's callback when
/// one is registered. Centralised so call sites stay symmetrical with
/// `events.push(...)` and never accidentally fire out of order.
fn fire_tool_event(cb: &Option<ToolEventCallback>, event: &MessageEvent) {
    if let Some(cb) = cb.as_ref() {
        cb(event);
    }
}

fn log_context(context: &ToolUseContext) -> LogContext {
    LogContext {
        session_id: context.session_id.clone(),
        correlation_id: context.correlation_id.clone(),
        agent_id: Some(context.agent_id.as_str().to_string()),
        agent_type: context.agent_type.clone(),
    }
}

/// Counters tracking how much streamed content actually flowed through
/// the wrapped callback during one turn. Used by the engine to decide
/// whether a defensive post-turn replay is necessary (i.e. the provider
/// silently fell back to a non-streaming call).
struct DeltaTracker {
    text_chars: AtomicUsize,
    thinking_chars: AtomicUsize,
}

/// Wrap a [`TextDeltaCallback`] so we can observe whether the provider
/// invoked it at all on a given turn. The returned `Arc<DeltaTracker>`
/// is read once the turn completes.
fn wrap_tracking_callback(inner: TextDeltaCallback) -> (TextDeltaCallback, Arc<DeltaTracker>) {
    let tracker = Arc::new(DeltaTracker {
        text_chars: AtomicUsize::new(0),
        thinking_chars: AtomicUsize::new(0),
    });
    let t = tracker.clone();
    let wrapped: TextDeltaCallback = Arc::new(move |delta| {
        if let Some(s) = delta.text.as_deref() {
            t.text_chars.fetch_add(s.chars().count(), Ordering::Relaxed);
        }
        if let Some(s) = delta.thinking.as_deref() {
            t.thinking_chars
                .fetch_add(s.chars().count(), Ordering::Relaxed);
        }
        inner(delta);
    });
    (wrapped, tracker)
}

/// Stop text for a run that used up its tool-call budget.
///
/// The loop only ends on its own when the model answers without calling a tool,
/// so a model stuck re-reading the same files would otherwise bill the user for
/// full-context requests until they notice and hit cancel. Whatever prose the
/// last turn produced is kept — it is usually a partial answer worth reading.
fn turn_limit_notice(limit: u32, partial: Option<&str>) -> String {
    let notice = format!(
        "[已达到本轮最大工具调用次数上限（{limit} 次），自动停止。\
         如果任务确实需要更多步骤，请拆分任务，或在 agent 配置中调高 maxTurns。]"
    );
    match partial.map(str::trim).filter(|s| !s.is_empty()) {
        Some(text) => format!("{text}\n\n{notice}"),
        None => notice,
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
                description: crate::ai::agent::tools::resolved_tool_description(
                    &s.name,
                    &s.description,
                ),
                schema: crate::ai::chat::with_declared_property_order(&s.schema),
            }
        })
        .collect()
}

/// True when this request resumes a server-side conversation, in which case
/// the provider serialiser omits the tool list entirely.
fn continues_cache_chain(previous_response_id: Option<&str>) -> bool {
    previous_response_id
        .map(str::trim)
        .is_some_and(|id| !id.is_empty())
}

/// Two-stage tool exposure for "anchored" agents.
///
/// A model's first request decides which behavioural policy it settles into
/// for the rest of the run, and a large tool catalog pulls some models into a
/// verbose, step-narrating style. Advertising a single tool up front keeps
/// that first decision in the region a small-scaffold prompt was trained on;
/// once the model has answered with a tool call the full catalog goes back on
/// the wire, so nothing is actually taken away from it.
///
/// This is transport staging only. The [`ToolPool`] is never narrowed, so a
/// model that calls a stage-2 tool during stage 1 is still served normally.
struct ToolAnchor {
    full: Vec<crate::ai::chat::ToolDefinition>,
    /// The stage-1 catalog, or `None` once the full catalog has been released
    /// (and when the anchor was a no-op to begin with).
    staged: Option<Vec<crate::ai::chat::ToolDefinition>>,
}

impl ToolAnchor {
    /// Names absent from `full` are ignored. An anchor that selects nothing,
    /// or that already covers the whole catalog, degrades to no anchoring —
    /// staging either would only cost a cache miss for no change in what the
    /// model sees.
    fn new(full: Vec<crate::ai::chat::ToolDefinition>, anchor: &[String]) -> Self {
        let wanted: std::collections::HashSet<&str> = anchor.iter().map(String::as_str).collect();
        let staged: Vec<_> = full
            .iter()
            .filter(|d| wanted.contains(d.name.as_str()))
            .cloned()
            .collect();
        let staged = (!staged.is_empty() && staged.len() < full.len()).then_some(staged);
        Self { full, staged }
    }

    /// Catalog for the first request.
    fn initial(&self) -> Vec<crate::ai::chat::ToolDefinition> {
        self.staged.clone().unwrap_or_else(|| self.full.clone())
    }

    /// Whether this anchor actually narrows anything.
    fn is_staged(&self) -> bool {
        self.staged.is_some()
    }

    /// Hand back the full catalog, exactly once.
    fn release(&mut self) -> Option<Vec<crate::ai::chat::ToolDefinition>> {
        self.staged.take().map(|_| self.full.clone())
    }
}

/// Accumulate token usage across multiple API turns (tool-call rounds).
///
/// Unlike `merge_usage` (which replaces), this ADDS prompt/completion tokens so
/// the final `TokenUsage` reflects the true total cost of the entire agent loop.
fn accumulate_usage(
    target: &mut crate::ai::tokens::TokenUsage,
    next: &crate::ai::tokens::TokenUsage,
) {
    if let Some(p) = next.prompt_tokens {
        *target.prompt_tokens.get_or_insert(0) += p;
        // Track (not sum) the latest round's prompt size. The last API call of a
        // turn already carries the full history, so this is the real
        // context-window occupancy — used by the composer context ring.
        target.last_prompt_tokens = Some(p);
    }
    if let Some(c) = next.completion_tokens {
        *target.completion_tokens.get_or_insert(0) += c;
    }
    if let Some(r) = next.cache_read_tokens {
        *target.cache_read_tokens.get_or_insert(0) += r;
    }
    if let Some(w) = next.cache_write_tokens {
        *target.cache_write_tokens.get_or_insert(0) += w;
    }
    // Prefer summing provider totals (already consistent per-provider). Fallback
    // to prompt+completion when a round omits total.
    if let Some(t) = next.total_tokens {
        *target.total_tokens.get_or_insert(0) += t;
    } else {
        target.total_tokens = match (target.prompt_tokens, target.completion_tokens) {
            (Some(p), Some(c)) => Some(p + c),
            (Some(p), None) => Some(p),
            (None, Some(c)) => Some(c),
            (None, None) => target.total_tokens,
        };
    }
}

/// Move the in-flight assistant/tool-result pair into `tool_chain`.
fn commit_tool_round(chat: &mut ChatRequest) {
    let Some(pending) = chat.pending_assistant_turn.take() else {
        return;
    };
    if pending.tool_calls.is_empty() && chat.tool_results.is_empty() {
        return;
    }
    chat.tool_chain.push(crate::ai::chat::ToolChainRound {
        assistant: pending,
        results: std::mem::take(&mut chat.tool_results),
    });
}

/// Append attachments as hidden user-meta history turns so the model
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
            thinking_content: None,
            timeline: Vec::new(),
        });
        // Bookkeeping: mark notification-shaped attachments rendered so
        // they don't get re-drained next time.
        if let AttachmentKind::TaskNotification(_) = &att.kind {
            // No-op today; placeholder for richer dedupe.
        }
    }
    // Place attachments at the *end* of history, immediately before the user
    // prompt. These arrive mid-session (task notifications, skill cites,
    // nested memory), so putting them up front would shift every prior turn
    // and void the provider's context cache for the whole transcript. Sitting
    // last also puts them closest to the prompt they relate to.
    chat.history.append(&mut injected);
}

/// Resolve `@skill:{"id":…}` cites in the user prompt and inject skill bodies.
pub fn inject_skill_cites_from_prompt(
    app: &tauri::AppHandle,
    chat: &mut ChatRequest,
    prompt: &str,
    enabled_skill_ids: &[String],
) {
    let Ok(resolved) =
        crate::data::skills::resolve_invoked_from_prompt(app, prompt, enabled_skill_ids)
    else {
        return;
    };
    if resolved.is_empty() {
        return;
    }
    let attachments: Vec<Attachment> = resolved
        .into_iter()
        .map(|(name, body)| Attachment::for_main(AttachmentKind::InvokedSkill { name, body }))
        .collect();
    inject_attachments_into_history(chat, &attachments);
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

#[cfg(test)]
mod tool_anchor_tests {
    use super::ToolAnchor;
    use crate::ai::chat::ToolDefinition;

    /// The `general-purpose` catalog as `collect_tool_definitions` produces
    /// it: every registered tool, sorted by name.
    const STANDARD: &[&str] = &[
        "Agent",
        "AskUser",
        "Bash",
        "ConsultRoles",
        "CreateDoc",
        "Delete",
        "Edit",
        "Grep",
        "ListFiles",
        "NovelAI",
        "Read",
        "RoleState",
        "TodoList",
        "WebFetch",
        "WebSearch",
        "Write",
    ];

    fn catalog(names: &[&str]) -> Vec<ToolDefinition> {
        names
            .iter()
            .map(|n| ToolDefinition {
                name: (*n).to_string(),
                description: String::new(),
                schema: serde_json::json!({}),
            })
            .collect()
    }

    fn names(defs: &[ToolDefinition]) -> Vec<String> {
        defs.iter().map(|d| d.name.clone()).collect()
    }

    fn read_anchor() -> Vec<String> {
        vec!["Read".to_string()]
    }

    /// The whole point of the anchored mode: whatever else is registered, the
    /// opening request puts one tool on the wire.
    #[test]
    fn first_request_advertises_the_anchor_alone() {
        let anchor = ToolAnchor::new(catalog(STANDARD), &read_anchor());
        assert!(anchor.is_staged());
        assert_eq!(names(&anchor.initial()), vec!["Read"]);
    }

    #[test]
    fn the_full_catalog_comes_back_after_the_first_tool_round() {
        let mut anchor = ToolAnchor::new(catalog(STANDARD), &read_anchor());
        let released = anchor.release().expect("first round restores every tool");
        assert_eq!(names(&released), STANDARD);
    }

    /// Releasing is a one-shot handover. Rewriting `chat.tools` on every
    /// later round would keep re-sending an identical array — harmless in
    /// content, but it re-enters the Volcengine chain-head path each time.
    #[test]
    fn later_rounds_leave_the_catalog_alone() {
        let mut anchor = ToolAnchor::new(catalog(STANDARD), &read_anchor());
        anchor.release();
        assert!(anchor.release().is_none());
    }

    /// `FileRead` is the name the prompts use; the registered tool is `Read`.
    /// A typo like that must not silently ship an empty tool array.
    #[test]
    fn an_anchor_that_matches_no_registered_tool_is_ignored() {
        let anchor = ToolAnchor::new(catalog(STANDARD), &["FileRead".to_string()]);
        assert!(!anchor.is_staged());
        assert_eq!(names(&anchor.initial()), STANDARD);
    }

    #[test]
    fn agents_without_an_anchor_are_untouched() {
        let anchor = ToolAnchor::new(catalog(STANDARD), &[]);
        assert!(!anchor.is_staged());
        assert_eq!(names(&anchor.initial()), STANDARD);
    }

    /// Staging the entire catalog changes nothing the model can see, so it
    /// should not trigger the mid-run catalog swap (and its cache miss).
    #[test]
    fn anchoring_everything_is_not_anchoring() {
        let all: Vec<String> = STANDARD.iter().map(|s| s.to_string()).collect();
        assert!(!ToolAnchor::new(catalog(STANDARD), &all).is_staged());
    }

    /// A narrowed agent (`disallowedTools`, per-node overrides) reaches the
    /// engine with a smaller pool; stage 2 must restore *that* pool, never
    /// the globally registered set.
    #[test]
    fn release_restores_the_pool_the_agent_was_filtered_for() {
        let filtered = &["Bash", "Grep", "Read"];
        let mut anchor = ToolAnchor::new(catalog(filtered), &read_anchor());
        assert_eq!(names(&anchor.initial()), vec!["Read"]);
        assert_eq!(names(&anchor.release().unwrap()), filtered);
    }
}

#[cfg(test)]
mod cache_chain_tests {
    use super::continues_cache_chain;

    /// Every user turn after the first restores the session's chain tip, and
    /// a continuation request carries no tool list — so staging one there
    /// would send nothing and only cost a cache miss when it was released.
    #[test]
    fn a_stored_chain_tip_counts_as_resuming() {
        assert!(continues_cache_chain(Some("resp_abc")));
    }

    #[test]
    fn a_fresh_session_does_not() {
        assert!(!continues_cache_chain(None));
        assert!(!continues_cache_chain(Some("")));
        assert!(!continues_cache_chain(Some("   ")));
    }
}

#[cfg(test)]
mod builtin_anchor_tests {
    use crate::ai::agent::config::builtin::{builtin_definitions, AGENT_ANCHORED};

    /// `anchored` only differs from `general-purpose` in how the catalog is
    /// staged — it must keep full tool access, or stage 2 restores nothing.
    #[test]
    fn the_anchored_agent_stages_read_over_a_full_pool() {
        let def = builtin_definitions()
            .into_iter()
            .find(|d| d.agent_type == AGENT_ANCHORED)
            .expect("anchored is registered as a built-in");
        assert_eq!(def.tools, vec!["*".to_string()]);
        assert_eq!(def.anchor_tools, vec!["Read".to_string()]);
        assert!(def.disallowed_tools.is_empty());
    }

    /// The short opening tool list reads as "your tools were taken away"
    /// unless the prompt says otherwise.
    #[test]
    fn the_anchored_prompt_explains_the_short_tool_list() {
        let def = builtin_definitions()
            .into_iter()
            .find(|d| d.agent_type == AGENT_ANCHORED)
            .unwrap();
        assert!(def.system_prompt.contains("Tool availability:"));
        assert!(def
            .system_prompt
            .contains(crate::ai::agent::config::prompts::GENERAL_PURPOSE_PROMPT));
    }
}

#[cfg(test)]
mod turn_limit_tests {
    use super::turn_limit_notice;

    #[test]
    fn partial_prose_is_kept_above_the_notice() {
        let out = turn_limit_notice(60, Some("第三章我已经写到一半"));
        assert!(out.starts_with("第三章我已经写到一半"));
        assert!(out.contains("60"));
    }

    #[test]
    fn a_turn_that_only_called_tools_still_explains_itself() {
        let out = turn_limit_notice(60, None);
        assert!(!out.trim().is_empty());
        assert!(out.contains("60"));
    }

    #[test]
    fn blank_prose_does_not_leave_leading_whitespace() {
        assert_eq!(
            turn_limit_notice(5, Some("   \n")),
            turn_limit_notice(5, None)
        );
    }
}
