use std::collections::HashMap;
use std::sync::{Arc, MutexGuard};

use tauri::{AppHandle, Emitter};

use crate::ai::agent::core::context::{AbortHandle, AbortSignal};
use crate::ai::agent::exec::query::ToolEventCallback;
use crate::ai::agent::memory::UserContextLoader;
use crate::ai::agent::{self, RunAgentParams, TaskState, ToolPool};
use crate::ai::{chat, parameters, router};
use crate::data::{paths, session, settings};
use crate::error::{AppError, AppResult};

use crate::app::history::append_role_state_history_tail;
use crate::app::project_rules;
use crate::app::state::AppState;

use super::params::{resolve_generation_definition, stage_display_name};
use super::streaming::StreamBlocks;

pub(crate) fn generation_abort_lock(
    state: &AppState,
) -> AppResult<MutexGuard<'_, HashMap<String, AbortHandle>>> {
    state
        .generation_abort
        .lock()
        .map_err(|_| AppError::Other("generation abort lock poisoned".into()))
}

/// Register a session-scoped abort controller and return the matching signal
/// for the agent run. Repeated cancel clicks call [`AbortHandle::abort`] on the
/// stored handle until the run finishes and the slot is cleared.
pub(crate) fn register_generation_abort(state: &AppState, session_id: &str) -> AppResult<AbortSignal> {
    let (signal, handle) = AbortSignal::new();
    let mut guard = generation_abort_lock(state)?;
    if guard.contains_key(session_id) {
        return Err(AppError::Invalid(
            "generation already in progress for session".into(),
        ));
    }
    guard.insert(session_id.to_string(), handle);
    Ok(signal)
}

pub(crate) fn clear_generation_abort(state: &AppState, session_id: &str) {
    if let Ok(mut guard) = state.generation_abort.lock() {
        guard.remove(session_id);
    }
}

/// Build the prompt handed to a downstream (N>1) agent flow stage. Wraps the
/// original user request together with the upstream stage's final output so
/// each stage refines the previous stage's result.
pub(crate) fn build_chain_stage_prompt(user_prompt: &str, prev_output: &str) -> String {
    format!(
        "You are a stage in an ordered agent pipeline. Continue the work by \
processing the previous agent's output.\n\n\
--- ORIGINAL USER REQUEST ---\n{user_prompt}\n\n\
--- PREVIOUS AGENT OUTPUT ---\n{prev_output}\n\n\
--- YOUR TASK ---\nProcess the previous agent's output according to your role \
and produce the refined result."
    )
}

/// Record an `agent_stage` marker block into the shared stream buffer so the
/// persisted assistant message keeps stage boundaries.
pub(crate) fn push_agent_stage_block(blocks: &StreamBlocks, agent_type: &str, name: &str, index: usize) {
    if let Ok(mut g) = blocks.lock() {
        g.push(serde_json::json!({
            "type": "agent_stage",
            "agent_type": agent_type,
            "name": name,
            "index": index,
        }));
    }
}

/// Emit a live `agent_stage` event so the UI can render the stage separator
/// while the chain streams.
pub(crate) fn emit_agent_stage(
    app: &AppHandle,
    session_id: &str,
    request_message_id: &str,
    agent_type: &str,
    name: &str,
    index: usize,
) {
    let _ = app.emit(
        "gen://stream",
        serde_json::json!({
            "session_id": session_id,
            "request_message_id": request_message_id,
            "stage": {
                "agent_type": agent_type,
                "name": name,
                "index": index,
            },
        }),
    );
}

/// Run an ordered agent flow chain for a single user turn.
///
/// Stage 0 receives the prepared `base_request` (full history + user prompt +
/// attachments). Each later stage gets a fresh request whose prompt wraps the
/// original user prompt plus the previous stage's final text (see
/// [`build_chain_stage_prompt`]), so the chain behaves like a streaming
/// pipeline (main -> state-machine -> fixer -> ...). All stages stream into the
/// same `stream_blocks`; an `agent_stage` marker is pushed and emitted before
/// each stage. Returns the merged response: the last stage's text/thinking/
/// usage plus images collected across every stage.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_agent_chain(
    state: &AppState,
    app: &AppHandle,
    session_id: &str,
    request_message_id: &str,
    chain: &[session::ChainNode],
    main_agent: &str,
    user_prompt: &str,
    base_request: chat::ChatRequest,
    provider: &settings::ModelProvider,
    model: &str,
    session_prompt: &str,
    params: &parameters::GenerationParameters,
    project_cwd: Option<std::path::PathBuf>,
    stream_blocks: &StreamBlocks,
    on_text_delta: chat::TextDeltaCallback,
    on_tool_event: ToolEventCallback,
) -> AppResult<chat::GenerateResponse> {
    let mut base_request = Some(base_request);
    let mut prev_text: Option<String> = None;
    let mut merged = chat::GenerateResponse::default();

    for (idx, node) in chain.iter().enumerate() {
        // The default main agent is referenced by a sentinel so it tracks the
        // session's `agent_type` (general-purpose / Plan) wherever it sits.
        let raw_type = node.agent_type.as_str();
        let is_main = raw_type == session::AGENT_CHAIN_MAIN;
        let agent_type: &str = if is_main { main_agent } else { raw_type };
        // Per-node config overrides apply only to this chain position.
        let overrides = node.effective_overrides();
        let name = if is_main {
            main_agent.to_string()
        } else {
            stage_display_name(state, agent_type)
        };
        push_agent_stage_block(stream_blocks, agent_type, &name, idx);
        emit_agent_stage(app, session_id, request_message_id, agent_type, &name, idx);

        let (request, stage_prompt) = if idx == 0 {
            let r = base_request.take().expect("stage 0 request present");
            let p = r.prompt.clone();
            (r, p)
        } else {
            let wrapped = build_chain_stage_prompt(user_prompt, prev_text.as_deref().unwrap_or(""));
            let r = router::build_chat_request(
                provider,
                model,
                wrapped.clone(),
                Vec::new(),
                session_prompt.to_string(),
                Vec::new(),
                params.clone(),
            )?;
            (r, wrapped)
        };

        // Side-effect-only stages (e.g. the `role-state` character state
        // machine) must not clobber the prose carried down the chain. Their
        // own reply is discarded; only token usage and images accumulate.
        let passthrough = resolve_generation_definition(state, agent_type)
            .map(|d| d.passthrough_output)
            .unwrap_or(false);

        // Passthrough stages keep their tool events (so RoleState updates
        // stream to the UI) but suppress their text deltas so the state
        // machine's terse reply never lands in the chat transcript.
        let stage_text_delta = if passthrough {
            None
        } else {
            Some(on_text_delta.clone())
        };

        let resp = run_cancellable_generation(
            state,
            session_id,
            agent_type,
            stage_prompt,
            request,
            stage_text_delta,
            Some(on_tool_event.clone()),
            project_cwd.clone(),
            overrides,
            Some(request_message_id),
        )
        .await?;

        merged.usage = resp.usage;
        merged.images.extend(resp.images);
        merged.videos.extend(resp.videos);
        if resp.response_id.is_some() {
            merged.response_id = resp.response_id;
        }
        if !passthrough {
            prev_text = resp.text.clone();
            merged.text = resp.text;
            merged.thinking_content = resp.thinking_content;
        }
    }

    Ok(merged)
}

pub(crate) async fn run_cancellable_generation(
    state: &AppState,
    session_id: &str,
    agent_type: &str,
    prompt: String,
    mut request: chat::ChatRequest,
    on_text_delta: Option<chat::TextDeltaCallback>,
    on_tool_event: Option<ToolEventCallback>,
    project_cwd: Option<std::path::PathBuf>,
    overrides: Option<&session::NodeOverrides>,
    correlation_id: Option<&str>,
) -> AppResult<chat::GenerateResponse> {
    let abort_signal = register_generation_abort(state, session_id)?;

    // Drain any pending task-notifications addressed to the main loop and
    // append them to the chat history as hidden user-meta turns. This
    // mirrors how `query.ts` injects `<task-notification>` at turn
    // boundaries so the model sees background results on the *next* call.
    let drained = state.notifications.drain_for_main();
    if !drained.is_empty() {
        crate::ai::agent::exec::engine::inject_attachments_into_history(&mut request, &drained);
    }

    let mut definition = resolve_generation_definition(state, agent_type)?;

    // Apply this chain node's per-node overrides on top of the resolved
    // definition. The global built-in / custom agent stays untouched; only this
    // run sees the overridden prompt / model / tools.
    if let Some(ov) = overrides {
        if let Some(sp) = &ov.system_prompt {
            definition.system_prompt = sp.clone();
        }
        if let Some(m) = &ov.model {
            let trimmed = m.trim();
            definition.model = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Some(tools) = &ov.tools {
            // Per-node override tool semantics:
            //   ["*"]   → all (non-denied) tools — only when the base definition
            //             already allows `*`. Whitelist agents (e.g. main `chat`)
            //             must not be expandable via a stale `["*"]` override.
            //   [names] → exactly those tools, intersected with the base allow
            //             list when the base is not `*`.
            //   []      → NO tools (empty allow-list; the agent runs with zero
            //             tools). This lets a node — including the main agent —
            //             be configured for pure generation without tool access.
            let base_wildcard = definition.tools.iter().any(|t| t == "*");
            if tools.iter().any(|t| t == "*") {
                if !base_wildcard {
                    // Keep the definition's whitelist (no expansion).
                } else {
                    definition.tools = tools.clone();
                }
            } else if base_wildcard {
                definition.tools = tools.clone();
            } else {
                let allow: std::collections::HashSet<&str> =
                    definition.tools.iter().map(|s| s.as_str()).collect();
                definition.tools = tools
                    .iter()
                    .filter(|t| allow.contains(t.as_str()))
                    .cloned()
                    .collect();
            }
        }
    }

    // Prepend user-context when the agent opts in (Plan/Explore omit it).
    if let Ok(ctx) = state.user_context.load() {
        if !definition.omit_claude_md {
            let rendered = ctx.rendered.trim();
            if !rendered.is_empty() {
                let mut head = vec![chat::HistoryTurn {
                    role: "user".into(),
                    text: Some(ctx.rendered.clone()),
                    thinking_content: None,
                    images: Vec::new(),
                    timeline: Vec::new(),
                }];
                head.append(&mut request.history);
                request.history = head;
            }
        }
    }

    // Append the structured role board after the transcript. The dedicated
    // `role-state` sub-agent reads prose + calls `RoleState` get instead.
    if agent_type != crate::ai::agent::config::builtin::AGENT_ROLE_STATE {
        append_role_state_history_tail(state, session_id, &mut request.history)?;
    }

    // Merge per-session system instructions into the definition body so
    // `compose_system_prompt` sees one cohesive instruction block.
    let session_sys = std::mem::take(&mut request.system_prompt);
    let session_sys = session_sys.trim();
    if !session_sys.is_empty() {
        let base = definition.system_prompt.trim();
        definition.system_prompt = if base.is_empty() {
            session_sys.to_string()
        } else {
            format!("{base}\n\n---\n\n{session_sys}")
        };
    }

    // Inject enabled project rules (`<projectRoot>/.moyan/*.md`) so they read as
    // part of the system prompt on every generation.
    if let Some(cwd) = project_cwd.as_deref() {
        if let Some(rules) = project_rules::collect_project_rules(cwd) {
            let base = definition.system_prompt.trim();
            definition.system_prompt = if base.is_empty() {
                rules
            } else {
                format!("{base}\n\n---\n\n{rules}")
            };
        }
    }

    let worker = ToolPool::new();
    for (_, tool) in state
        .tools
        .filter_for_agent(&definition.tools, &definition.disallowed_tools)
    {
        worker.register_arc(tool);
    }
    let worker_tools = Arc::new(worker);

    let query_engine = state.query_engine.clone();
    let task_store = state.task_store.clone();
    let role_state_scope_id = {
        let conn = state.conn()?;
        crate::data::role_state::resolve_role_state_scope(&conn, session_id)?
    };

    let outcome = tokio::select! {
        out = agent::run_agent(RunAgentParams {
            definition,
            prompt,
            run_mode: agent::AgentRunMode::Foreground,
            chat_request: request,
            tools: worker_tools,
            task_store,
            engine: query_engine,
            initial_attachments: Vec::new(),
            permission_override: None,
            parent_system_prompt: None,
            on_text_delta,
            on_tool_event,
            query_source: Some(agent::QuerySource::ReplMainThread),
            project_cwd,
            abort_signal: Some(abort_signal.clone()),
            session_id: Some(session_id.to_string()),
            role_state_scope_id: Some(role_state_scope_id),
            correlation_id: correlation_id.map(str::to_string),
            token_stats: Some(state.token_stats.clone()),
            session_logger: Some(state.session_logger.clone()),
        }) => out,
        _ = abort_signal.wait_aborted() => Err(AppError::Canceled),
    };
    clear_generation_abort(state, session_id);

    let run = outcome?;
    Ok(chat::GenerateResponse {
        images: run.images,
        videos: run.videos,
        text: run.final_text,
        thinking_content: run.thinking_content,
        usage: run.usage,
        tool_calls: Vec::new(),
        response_id: run.response_id,
    })
}

pub(crate) fn maybe_extract_session_memory(
    state: &AppState,
    app: &AppHandle,
    session_id: &str,
    usage: &crate::ai::tokens::TokenUsage,
) {
    if !state.session_memory.should_update(usage, 0) {
        return;
    }
    let Ok(dir) = paths::session_dir(app, session_id) else {
        return;
    };
    let latest = state
        .task_store
        .list()
        .into_iter()
        .filter(|t| !matches!(t.state, TaskState::Pending | TaskState::Running))
        .max_by_key(|t| t.ended_at_ms.unwrap_or(t.started_at_ms));
    let _ = state
        .session_memory
        .extract_now(session_id, &dir, latest.as_ref());
}
