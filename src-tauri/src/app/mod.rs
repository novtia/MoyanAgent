use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use crate::ai::agent::core::context::{AbortHandle, AbortSignal};

use crate::ai::agent::config::mcp::McpRegistry;
use crate::ai::agent::tools::agent_tool::{AgentTool, ChatRequestFactory};
use crate::ai::agent::exec::engine::ProviderQueryEngine;
use crate::ai::agent::exec::query::ToolEventCallback;
use crate::ai::agent::memory::UserContextLoader;
use crate::ai::agent::types::MessageEvent;
use crate::ai::agent::{
    self, AgentRegistry, FileReadTool, FileSnapshotStore, FsSessionMemoryExtractor,
    FsUserContextLoader, NotificationQueue, ProviderEngine, QueryEngine, RoleStateStore,
    RoleStateTool, RunAgentParams, StaticMcpRegistry, Task, TaskState, TaskStore, ToolPool,
    UserContextConfig,
};
use crate::ai::{chat, parameters, router, token_log};
use crate::data::db::DbPool;
use crate::data::{custom_agents, db, llm_catalog, paths, project, session, settings};
use crate::ai::agent::tools::text_decode::{
    read_text_file, write_text_file_labeled, ProjectTextFile, TextEncoding,
};
use crate::error::{AppError, AppResult};
use crate::media::{editor, images};

mod project_fs;
mod project_rules;

pub struct AppState {
    pub pool: DbPool,
    /// Per-session abort controllers for in-flight `generate_image` runs.
    generation_abort: Mutex<HashMap<String, AbortHandle>>,

    /// Agent subsystem services. Kept on `AppState` so that any Tauri
    /// command can register tasks, fan out notifications, or pick up a
    /// definition without walking the global registry every call.
    pub agent_registry: Arc<AgentRegistry>,
    pub task_store: Arc<TaskStore>,
    pub notifications: Arc<NotificationQueue>,
    #[allow(dead_code)]
    pub engine: Arc<ProviderEngine>,
    /// Full agent query loop (tool turns). Shared with [`AgentTool`].
    pub query_engine: Arc<dyn QueryEngine>,
    /// CLAUDE.md / user-context loader; cached, invalidate on compact.
    pub user_context: Arc<FsUserContextLoader>,
    /// MCP registry snapshot used by `AgentTool` to gate sub-agents.
    pub mcp: Arc<StaticMcpRegistry>,
    /// Shared tool pool. Currently holds [`FileReadTool`]; further
    /// host-implemented tools register on top.
    pub tools: Arc<ToolPool>,
    /// Per-session-memory extractor. Stateless aside from the last
    /// observed [`SessionMemory`] snapshot.
    pub session_memory: Arc<FsSessionMemoryExtractor>,
    /// Shared, project/session-scoped character state board mutated by the
    /// `RoleState` tool and snapshotted per assistant message.
    pub role_states: Arc<RoleStateStore>,
    /// Shared, session-scoped buffer of pending agent file mutations. Drained
    /// per assistant message into `file_snapshots` for delete/regenerate
    /// rollback of created / updated / deleted files.
    pub file_snapshots: Arc<FileSnapshotStore>,
    /// Structured token usage logger (JSONL + SQLite).
    pub token_logger: Arc<token_log::TokenUsageLogger>,
}

impl AppState {
    fn conn(&self) -> AppResult<db::DbConn> {
        Ok(self.pool.get()?)
    }
}

fn generation_abort_lock(
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
fn register_generation_abort(
    state: &AppState,
    session_id: &str,
) -> AppResult<AbortSignal> {
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

fn clear_generation_abort(state: &AppState, session_id: &str) {
    if let Ok(mut guard) = state.generation_abort.lock() {
        guard.remove(session_id);
    }
}

// #region agent log
fn agent_dbg(location: &str, message: &str, hypothesis: &str, data: serde_json::Value) {
    use std::io::Write;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("d:/AI/MoyanAgent/debug-2b2c78.log")
    {
        let _ = writeln!(
            f,
            "{}",
            serde_json::json!({"sessionId":"2b2c78","hypothesisId":hypothesis,"location":location,"message":message,"data":data,"timestamp":ts})
        );
    }
}

fn agent_dbg_has_marker(s: &str) -> bool {
    s.contains("已调用工具") || s.contains("[阶段")
}
// #endregion

/// Build prior-turn context for a session (oldest first), capped at `max_messages`.
/// Skips `error`-role messages and any drafts. When `before_ms` is `Some(t)`, only
/// messages with `created_at < t` are considered (used by regenerate to drop the
/// re-sent prompt and any stale assistant replies).
fn build_history(
    app: &AppHandle,
    conn: &db::DbConn,
    session_id: &str,
    before_ms: Option<i64>,
    max_messages: usize,
) -> AppResult<Vec<chat::HistoryTurn>> {
    if max_messages == 0 {
        return Ok(Vec::new());
    }
    let loaded = session::load_with_messages(conn, session_id)?;
    let candidates: Vec<&session::Message> = loaded
        .messages
        .iter()
        .filter(|m| matches!(m.role.as_str(), "user" | "assistant"))
        .filter(|m| match before_ms {
            Some(t) => m.created_at < t,
            None => true,
        })
        .filter(|m| message_qualifies_for_history(m))
        .collect();

    let len = candidates.len();
    let start = len.saturating_sub(max_messages);
    let mut out: Vec<chat::HistoryTurn> = Vec::with_capacity(len - start);
    for m in &candidates[start..] {
        let want_roles: &[&str] = match m.role.as_str() {
            "user" => &["input", "edited"],
            "assistant" => &["output", "edited"],
            _ => &[],
        };
        let mut imgs: Vec<&session::ImageRef> = m
            .images
            .iter()
            .filter(|i| want_roles.contains(&i.role.as_str()))
            .collect();
        imgs.sort_by_key(|i| i.ord);
        let mut payload: Vec<chat::AttachmentBytes> = Vec::with_capacity(imgs.len());
        for img in imgs {
            let bytes = images::read_image_bytes(app, img)?;
            payload.push(chat::AttachmentBytes {
                bytes,
                mime: img.mime.clone(),
            });
        }
        let thinking_content = message_thinking_for_history(m);
        let timeline = message_timeline_for_history(m);
        out.push(chat::HistoryTurn {
            role: m.role.clone(),
            text: history_text_for_message(m),
            images: payload,
            thinking_content,
            timeline,
        });
    }
    Ok(out)
}

/// Load the current character state board for prompt injection: prefer the
/// in-memory store, fall back to the latest persisted snapshot.
fn load_roles_for_prompt(state: &AppState, session_id: &str) -> AppResult<Vec<serde_json::Value>> {
    let conn = state.conn()?;
    let scope = crate::data::role_state::resolve_role_state_scope(&conn, session_id)?;
    let live = state.role_states.snapshot(&scope);
    if !live.is_empty() {
        return Ok(live);
    }
    crate::data::role_state::latest_roles(&conn, &scope)
}

/// Render the role board as a tagged user-meta block appended to history.
fn format_role_state_history_block(roles: &[serde_json::Value]) -> String {
    let json = serde_json::to_string_pretty(roles).unwrap_or_else(|_| "[]".to_string());
    format!(
        "<role-state>\n\
         当前角色状态板（JSON）。续写正文时请与此状态保持一致；女性 nsfw.semen 的 ml 字段请按故事尺度理解。\n\n\
         {json}\n\
         </role-state>"
    )
}

/// Append the latest role board as the final history turn so the model sees
/// structured character state after the conversational transcript.
fn append_role_state_history_tail(
    state: &AppState,
    session_id: &str,
    history: &mut Vec<chat::HistoryTurn>,
) -> AppResult<()> {
    let roles = load_roles_for_prompt(state, session_id)?;
    if roles.is_empty() {
        return Ok(());
    }
    history.push(chat::HistoryTurn {
        role: "user".into(),
        text: Some(format_role_state_history_block(&roles)),
        images: Vec::new(),
        thinking_content: None,
        timeline: Vec::new(),
    });
    Ok(())
}

/// Run the primary session through the full agent runtime ([`agent::run_agent`]
/// + [`ProviderQueryEngine`]): definition system prompt, tool loop, and task
/// tracking. The per-session abort controller races the run and returns
/// [`AppError::Canceled`], propagating cancellation through the tool context
/// so in-flight provider streams and tool calls can stop promptly.
/// Resolve the project working directory for a session, if any.
///
/// Returns `Some(path)` when the session belongs to a project that has a
/// non-empty `path` set; `None` otherwise (plain chat, or project without
/// a filesystem path).
pub(crate) fn session_project_cwd(conn: &db::DbConn, session_id: &str) -> Option<std::path::PathBuf> {
    let sess = session::get(conn, session_id).ok()?;
    let project_id = sess.project_id?;
    let proj = project::get(conn, &project_id).ok()?;
    let path_str = proj.path.filter(|p| !p.trim().is_empty())?;
    Some(std::path::PathBuf::from(path_str))
}

/// Allowed roots for user-initiated writes from the reader panel.
fn reader_write_roots(session_cwd: Option<&std::path::Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(cwd) = session_cwd {
        if cwd.is_absolute() {
            roots.push(cwd.to_path_buf());
        }
    }
    if let Ok(root) = paths::user_moyan_root() {
        roots.push(root);
    }
    if let Ok(root) = paths::blank_projects_root() {
        roots.push(root);
    }
    roots
}

fn path_under_root(path: &std::path::Path, root: &std::path::Path) -> bool {
    path.starts_with(root)
}

pub(crate) fn validate_reader_write_path(
    file_path: &std::path::Path,
    session_cwd: Option<&std::path::Path>,
) -> AppResult<PathBuf> {
    let canonical = std::fs::canonicalize(file_path).unwrap_or_else(|_| file_path.to_path_buf());
    let roots = reader_write_roots(session_cwd);
    if roots.is_empty() {
        return Err(AppError::Invalid(
            "write_project_file: no allowed write directory for this session".into(),
        ));
    }
    for root in &roots {
        let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        if path_under_root(&canonical, &root_canon) {
            return Ok(canonical);
        }
    }
    Err(AppError::Invalid(format!(
        "write_project_file: path {:?} is outside the project or allowed documents folder",
        file_path.display()
    )))
}

#[tauri::command]
fn write_project_file(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    path: String,
    content: String,
    encoding: Option<String>,
    had_bom: Option<bool>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let file_path = PathBuf::from(&path);
    let cwd = session_project_cwd(&conn, &session_id);
    let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;

    write_text_file_labeled(&resolved, &content, encoding.as_deref(), had_bom).map_err(|e| {
        AppError::Other(format!("write_project_file: write {:?}: {e}", resolved))
    })?;
    Ok(())
}

#[tauri::command]
fn read_project_file(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    path: String,
) -> Result<ProjectTextFile, AppError> {
    let conn = state.conn()?;
    let file_path = PathBuf::from(&path);
    let cwd = session_project_cwd(&conn, &session_id);
    let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;
    read_text_file(&resolved)
        .map(ProjectTextFile::from)
        .map_err(|e| AppError::Other(format!("read_project_file: read {:?}: {e}", resolved)))
}

/// Effective generation parameters for a session.
///
/// If the session belongs to a project, the project's shared parameters are
/// used (system prompt, history turns, LLM params, context window override).
/// Otherwise the session's own parameters apply.
#[allow(dead_code)]
struct EffectiveSessionParams {
    pub system_prompt: String,
    pub history_turns: i64,
    pub llm_params: settings::ModelParamSettings,
    /// Context window override; reserved for future use in compaction / UI hints.
    pub context_window: Option<i64>,
}

fn effective_session_params(
    conn: &db::DbConn,
    sess: &session::Session,
) -> EffectiveSessionParams {
    if let Some(ref pid) = sess.project_id {
        if let Ok(proj) = project::get(conn, pid) {
            return EffectiveSessionParams {
                system_prompt: proj.system_prompt,
                history_turns: proj.history_turns,
                llm_params: proj.llm_params,
                context_window: proj.context_window.or(sess.context_window),
            };
        }
    }
    EffectiveSessionParams {
        system_prompt: sess.system_prompt.clone(),
        history_turns: sess.history_turns,
        llm_params: sess.llm_params.clone(),
        context_window: sess.context_window,
    }
}

/// Resolve the agent flow chain that should drive a session's generation.
///
/// Sessions belonging to a project share the project's single agent flow
/// record, so editing the chain on any conversation applies to all of them and
/// new conversations inherit it. Plain (project-less) sessions keep their own
/// per-session chain.
fn effective_agent_chain(
    conn: &db::DbConn,
    sess: &session::Session,
) -> Option<Vec<session::ChainNode>> {
    if let Some(ref pid) = sess.project_id {
        if let Ok(proj) = project::get(conn, pid) {
            return proj.agent_chain;
        }
    }
    sess.agent_chain.clone()
}

/// Resolve the [`AgentDefinition`] that should drive a primary-session
/// generation for `agent_type`. Built-in agents come from the registry
/// (MCP-gated); user-defined agents (`custom:*`) are loaded from the
/// `custom_agents` table on demand.
fn resolve_generation_definition(
    state: &AppState,
    agent_type: &str,
) -> AppResult<crate::ai::agent::AgentDefinition> {
    let mcp_available = state.mcp.available_servers();
    if let Some(d) = state
        .agent_registry
        .filter_by_mcp(&mcp_available)
        .get(agent_type)
        .cloned()
    {
        return Ok(d);
    }
    if agent_type.starts_with(custom_agents::CUSTOM_AGENT_PREFIX) {
        let conn = state.conn()?;
        if let Some(ca) = custom_agents::get(&conn, agent_type)? {
            return Ok(ca.to_definition());
        }
    }
    Err(AppError::Invalid(format!(
        "unknown or MCP-unavailable agent type for main session: {agent_type}"
    )))
}

/// Human-friendly label for an agent flow stage. Custom agents show their
/// stored name; built-ins show the `agent_type` directly.
fn stage_display_name(state: &AppState, agent_type: &str) -> String {
    if agent_type.starts_with(custom_agents::CUSTOM_AGENT_PREFIX) {
        if let Ok(conn) = state.conn() {
            if let Ok(Some(ca)) = custom_agents::get(&conn, agent_type) {
                return ca.name;
            }
        }
    }
    agent_type.to_string()
}

/// Build the prompt handed to a downstream (N>1) agent flow stage. Wraps the
/// original user request together with the upstream stage's final output so
/// each stage refines the previous stage's result.
fn build_chain_stage_prompt(user_prompt: &str, prev_output: &str) -> String {
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
fn push_agent_stage_block(blocks: &StreamBlocks, agent_type: &str, name: &str, index: usize) {
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
fn emit_agent_stage(
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
async fn run_agent_chain(
    state: &AppState,
    app: &AppHandle,
    session_id: &str,
    request_message_id: &str,
    chain: &[session::ChainNode],
    main_agent: &str,
    user_prompt: &str,
    base_request: chat::ChatRequest,
    settings: &settings::Settings,
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
                settings,
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

        // #region agent log
        agent_dbg(
            "app/mod.rs:run_agent_chain",
            "stage start",
            "B/D",
            serde_json::json!({
                "idx": idx,
                "agent_type": agent_type,
                "passthrough": passthrough,
                "streams_text": !passthrough,
                "prev_len": prev_text.as_deref().map(|s| s.len()).unwrap_or(0),
                "prev_has_marker": prev_text.as_deref().map(agent_dbg_has_marker).unwrap_or(false),
            }),
        );
        // #endregion

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

        // #region agent log
        agent_dbg(
            "app/mod.rs:run_agent_chain",
            "stage done",
            "B/E",
            serde_json::json!({
                "idx": idx,
                "agent_type": agent_type,
                "text_len": resp.text.as_deref().map(|s| s.len()).unwrap_or(0),
                "text_has_marker": resp.text.as_deref().map(agent_dbg_has_marker).unwrap_or(false),
                "text_preview": resp.text.as_deref().map(|s| s.chars().take(160).collect::<String>()).unwrap_or_default(),
            }),
        );
        // #endregion
        merged.usage = resp.usage;
        merged.images.extend(resp.images);
        if !passthrough {
            prev_text = resp.text.clone();
            merged.text = resp.text;
            merged.thinking_content = resp.thinking_content;
        }
    }

    Ok(merged)
}

async fn run_cancellable_generation(
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
    // prepend them to the chat history as hidden user-meta turns. This
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
            //   ["*"]   → all (non-denied) tools
            //   [names] → exactly those tools
            //   []      → NO tools (empty allow-list; the agent runs with zero
            //             tools). This lets a node — including the main agent —
            //             be configured for pure generation without tool access.
            definition.tools = tools.clone();
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
            token_logger: Some(state.token_logger.clone()),
        }) => out,
        _ = abort_signal.wait_aborted() => Err(AppError::Canceled),
    };
    clear_generation_abort(state, session_id);

    let run = outcome?;
    Ok(chat::GenerateResponse {
        images: run.images,
        text: run.final_text,
        thinking_content: run.thinking_content,
        usage: run.usage,
        tool_calls: Vec::new(),
    })
}

/// Bridges [`ChatRequestFactory`] over the host's settings + db pool +
/// user-context loader so the model-callable `Agent` tool can synthesise
/// a sub-agent [`ChatRequest`] without the agent layer knowing anything
/// about SQLite, settings storage, or CLAUDE.md discovery.
struct SettingsChatFactory {
    pool: DbPool,
    user_context: Arc<FsUserContextLoader>,
}

impl SettingsChatFactory {
    fn new(pool: DbPool, user_context: Arc<FsUserContextLoader>) -> Self {
        Self { pool, user_context }
    }
}

impl ChatRequestFactory for SettingsChatFactory {
    fn build(
        &self,
        prompt: &str,
        _agent_type: &str,
        definition: &crate::ai::agent::AgentDefinition,
    ) -> AppResult<(chat::ChatRequest, Vec<agent::Attachment>)> {
        let conn = self.pool.get()?;
        let settings = settings::read(&conn)?;

        // Runner overwrites `system_prompt` with `definition.system_prompt`
        // plus env-details + critical reminder, so leave it empty here.
        let chat = crate::ai::router::build_chat_request(
            &settings,
            prompt.to_string(),
            Vec::new(),
            String::new(),
            Vec::new(),
            crate::ai::parameters::factory().build(
                String::new(),
                String::new(),
                Default::default(),
            ),
        )?;

        // Honour `omit_claude_md`: only inject user-context (CLAUDE.md +
        // rules) when the agent definition opts in. Rendered as a
        // `Delta { topic = "user_context" }` attachment so the engine
        // turns it into a `<system-reminder>` block on entry.
        let attachments = if definition.omit_claude_md {
            Vec::new()
        } else {
            self.user_context
                .load()
                .ok()
                .map(|uc| {
                    let rendered = uc.rendered.trim();
                    if rendered.is_empty() {
                        Vec::new()
                    } else {
                        vec![agent::Attachment::for_main(
                            agent::AttachmentKind::Delta {
                                topic: "user_context".into(),
                                body: rendered.to_string(),
                            },
                        )]
                    }
                })
                .unwrap_or_default()
        };

        Ok((chat, attachments))
    }
}

/// wants to refresh `summary.md` for this session. Mirrors the
/// `extractSessionMemory()` post-sampling pass - non-blocking, best-effort.
fn maybe_extract_session_memory(
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

/// Shared accumulator for ordered assistant blocks emitted while a
/// generation is in flight. Both [`stream_text_callback`] and
/// [`tool_event_callback`] mutate the same buffer so the final order
/// reflects exactly when text / thinking / tool events arrived from the
/// engine. The caller drains the buffer once generation finishes (or is
/// cancelled) to persist it onto the assistant message.
type StreamBlocks = Arc<Mutex<Vec<serde_json::Value>>>;

fn new_stream_blocks() -> StreamBlocks {
    Arc::new(Mutex::new(Vec::new()))
}

fn snapshot_stream_blocks(blocks: &StreamBlocks) -> Vec<serde_json::Value> {
    blocks
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default()
}

fn concat_block_text(blocks: &[serde_json::Value], block_type: &str) -> String {
    blocks
        .iter()
        .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some(block_type))
        .filter_map(|b| b.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

fn message_blocks(m: &session::Message) -> Option<&Vec<serde_json::Value>> {
    m.params
        .as_ref()
        .and_then(|p| p.get("blocks"))
        .and_then(|v| v.as_array())
}

/// Ordered timeline for a prior assistant turn, used by providers to
/// replay tool history in the native call/response format instead of a
/// leaked plain-text transcript. Prefers the persisted `params.timeline`
/// and falls back to reconstructing it from `params.blocks` for legacy
/// rows. User turns and turns without tool activity return an empty vec.
fn message_timeline_for_history(m: &session::Message) -> Vec<chat::TimelineSegment> {
    if m.role != "assistant" {
        return Vec::new();
    }
    if let Some(params) = m.params.as_ref() {
        if let Some(tv) = params.get("timeline") {
            if let Ok(segs) =
                serde_json::from_value::<Vec<chat::TimelineSegment>>(tv.clone())
            {
                if !segs.is_empty() {
                    return segs;
                }
            }
        }
    }
    if let Some(blocks) = message_blocks(m) {
        return crate::ai::block_timeline::restore_timeline_from_blocks(blocks);
    }
    Vec::new()
}

/// Visible assistant/user reply text for provider history. For assistant
/// turns this is the timeline's final `Text` segment (the model's actual
/// reply); tool transcripts are NEVER folded into text anymore. Falls
/// back to the persisted `text`/block text, always cleaned of any leaked
/// host tool-log lines so legacy rows don't re-teach the model to echo
/// them.
fn history_text_for_message(m: &session::Message) -> Option<String> {
    use crate::ai::stream_split::strip_leaked_host_tool_log;

    if m.role == "assistant" {
        let timeline = message_timeline_for_history(m);
        let summary = crate::ai::block_timeline::timeline_summary_text(&timeline);
        if !summary.is_empty() {
            return Some(summary);
        }
    }

    let mut parts: Vec<String> = Vec::new();
    if let Some(t) = m.text.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let cleaned = strip_leaked_host_tool_log(t);
        if !cleaned.trim().is_empty() {
            parts.push(cleaned.trim().to_string());
        }
    }
    if let Some(blocks) = message_blocks(m) {
        let block_text = strip_leaked_host_tool_log(concat_block_text(blocks, "text").trim());
        let block_text = block_text.trim().to_string();
        if !block_text.is_empty() && !parts.iter().any(|p| p.contains(&block_text)) {
            parts.push(block_text);
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

fn message_thinking_for_history(m: &session::Message) -> Option<String> {
    let mut thinking = m
        .params
        .as_ref()
        .and_then(|p| p.get("thinking_content"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_default();
    if let Some(blocks) = message_blocks(m) {
        let block_thinking = concat_block_text(blocks, "thinking").trim().to_string();
        if block_thinking.len() > thinking.len() {
            thinking = block_thinking;
        }
    }
    if thinking.is_empty() {
        None
    } else {
        Some(thinking)
    }
}

fn message_qualifies_for_history(m: &session::Message) -> bool {
    let has_text = history_text_for_message(m).is_some();
    let has_img = m
        .images
        .iter()
        .any(|i| matches!(i.role.as_str(), "input" | "output" | "edited"));
    if m.role == "assistant" {
        return has_text
            || has_img
            || message_thinking_for_history(m).is_some()
            || !message_timeline_for_history(m).is_empty();
    }
    has_text || has_img
}

/// Persist streamed assistant output before the session is reloaded.
///
/// On upstream failure the UI has already rendered deltas from `gen://stream`;
/// without this write, a DB reload would drop the in-flight assistant bubble and
/// leave only the separate `error` row.
fn persist_streamed_assistant_snapshot(
    conn: &db::DbConn,
    session_id: &str,
    blocks: &[serde_json::Value],
    fallback_text: Option<&str>,
    fallback_thinking: Option<&str>,
    mut params: serde_json::Value,
    file_snapshots: &FileSnapshotStore,
) -> AppResult<()> {
    use crate::ai::stream_split::strip_leaked_host_tool_log;

    session::get(conn, session_id)?;

    // Scrub any leaked host tool-log lines out of persisted text blocks so
    // an interrupted / errored partial snapshot can't re-teach the model to
    // echo them on the next turn.
    let cleaned_blocks: Vec<serde_json::Value> = blocks
        .iter()
        .map(|b| {
            if b.get("type").and_then(|v| v.as_str()) == Some("text") {
                if let Some(t) = b.get("content").and_then(|c| c.as_str()) {
                    let mut nb = b.clone();
                    if let Some(obj) = nb.as_object_mut() {
                        obj.insert(
                            "content".into(),
                            serde_json::Value::String(strip_leaked_host_tool_log(t)),
                        );
                    }
                    return nb;
                }
            }
            b.clone()
        })
        .collect();

    let mut text = concat_block_text(&cleaned_blocks, "text");
    if text.trim().is_empty() {
        text = fallback_text
            .map(strip_leaked_host_tool_log)
            .unwrap_or_default()
            .trim()
            .to_string();
    } else {
        text = text.trim().to_string();
    }
    let mut thinking = concat_block_text(&cleaned_blocks, "thinking");
    if thinking.trim().is_empty() {
        thinking = fallback_thinking.unwrap_or("").trim().to_string();
    } else {
        thinking = thinking.trim().to_string();
    }

    let has_blocks = !cleaned_blocks.is_empty();
    if text.is_empty() && thinking.is_empty() && !has_blocks {
        return Ok(());
    }

    if let Some(obj) = params.as_object_mut() {
        if !thinking.is_empty() {
            obj.insert(
                "thinking_content".into(),
                serde_json::Value::String(thinking.clone()),
            );
        }
        if has_blocks {
            let timeline =
                crate::ai::block_timeline::restore_timeline_from_blocks(&cleaned_blocks);
            if !timeline.is_empty() {
                if let Ok(tv) = serde_json::to_value(&timeline) {
                    obj.insert("timeline".into(), tv);
                }
            }
            obj.insert(
                "blocks".into(),
                serde_json::Value::Array(cleaned_blocks.clone()),
            );
        }
    }
    let params_json = params.to_string();
    let text_opt = if text.is_empty() {
        None
    } else {
        Some(text.as_str())
    };
    let assistant =
        session::insert_message(conn, session_id, "assistant", text_opt, Some(&params_json))?;

    // Bind any file mutations captured before the interrupt / error to this
    // partial message so they roll back if the message is deleted.
    let file_changes = file_snapshots.take(session_id);
    if !file_changes.is_empty() {
        let _ = crate::data::file_snapshot::save_changes(
            conn,
            session_id,
            &assistant.id,
            &file_changes,
        );
    }

    session::recompute_context_window_used(conn, session_id)?;
    Ok(())
}

/// Append a text delta to the ordered block list, merging with the
/// trailing block when it is also a `text` block.
fn append_text_delta_block(blocks: &mut Vec<serde_json::Value>, delta: &str) {
    if delta.is_empty() {
        return;
    }
    if let Some(last) = blocks.last_mut() {
        if last.get("type").and_then(|v| v.as_str()) == Some("text") {
            if let Some(content) = last.get_mut("content").and_then(|c| c.as_str()) {
                let merged = format!("{content}{delta}");
                if let Some(obj) = last.as_object_mut() {
                    obj.insert("content".into(), serde_json::Value::String(merged));
                }
                return;
            }
        }
    }
    blocks.push(serde_json::json!({ "type": "text", "content": delta }));
}

/// Same as [`append_text_delta_block`] but for `thinking` blocks.
fn append_thinking_delta_block(blocks: &mut Vec<serde_json::Value>, delta: &str) {
    if delta.is_empty() {
        return;
    }
    if let Some(last) = blocks.last_mut() {
        if last.get("type").and_then(|v| v.as_str()) == Some("thinking") {
            if let Some(content) = last.get_mut("content").and_then(|c| c.as_str()) {
                let merged = format!("{content}{delta}");
                if let Some(obj) = last.as_object_mut() {
                    obj.insert("content".into(), serde_json::Value::String(merged));
                }
                return;
            }
        }
    }
    blocks.push(serde_json::json!({ "type": "thinking", "content": delta }));
}

/// Push a new `tool_use` block in `pending` state.
fn record_tool_use_block(
    blocks: &mut Vec<serde_json::Value>,
    id: &str,
    tool: &str,
    input: &serde_json::Value,
) {
    blocks.push(serde_json::json!({
        "type": "tool_use",
        "id": id,
        "tool": tool,
        "input": input.clone(),
        "status": "pending",
    }));
}

/// Mutate the matching `tool_use` block in place with the tool result.
/// No-op if the matching id can't be found (defensive against duplicated
/// or out-of-order events).
fn record_tool_result_block(
    blocks: &mut Vec<serde_json::Value>,
    id: &str,
    output: &serde_json::Value,
    is_error: bool,
) {
    for b in blocks.iter_mut().rev() {
        if b.get("type").and_then(|v| v.as_str()) != Some("tool_use") {
            continue;
        }
        if b.get("id").and_then(|v| v.as_str()) != Some(id) {
            continue;
        }
        if let Some(obj) = b.as_object_mut() {
            obj.insert(
                "status".into(),
                serde_json::Value::String(
                    if is_error { "error" } else { "success" }.into(),
                ),
            );
            obj.insert("output".into(), output.clone());
            if is_error {
                obj.insert("is_error".into(), serde_json::Value::Bool(true));
            }
        }
        return;
    }
}

fn stream_text_callback(
    app: AppHandle,
    session_id: String,
    request_message_id: String,
    blocks: StreamBlocks,
) -> chat::TextDeltaCallback {
    // Per-request stateful cleaner: strips any host tool-transcript lines
    // (`[已调用工具 ...]`, `[阶段: ...]`) a model might echo, holding back
    // only a trailing fragment that could still become such a marker so
    // normal prose streams unimpeded. Shared (cloned) across chain stages.
    let splitter = Arc::new(std::sync::Mutex::new(
        crate::ai::stream_split::StreamContentSplitter::default(),
    ));
    Arc::new(move |delta| {
        // Route visible text through the marker cleaner before it reaches
        // either the persisted block buffer or the live UI stream.
        let cleaned_text = delta.text.as_deref().map(|t| {
            splitter
                .lock()
                .map(|mut s| s.push(t))
                .unwrap_or_else(|_| t.to_string())
        });
        // #region agent log
        if let Some(raw) = delta.text.as_deref() {
            if agent_dbg_has_marker(raw) {
                agent_dbg(
                    "app/mod.rs:stream_text_callback",
                    "marker text arrived in live provider stream delta",
                    "A/B/D",
                    serde_json::json!({
                        "session_id": session_id,
                        "request_message_id": request_message_id,
                        "raw_preview": raw.chars().take(200).collect::<String>(),
                        "cleaned_still_has_marker": cleaned_text.as_deref().map(agent_dbg_has_marker).unwrap_or(false),
                    }),
                );
            }
        }
        // #endregion
        if let Ok(mut g) = blocks.lock() {
            if let Some(t) = cleaned_text.as_deref() {
                append_text_delta_block(&mut g, t);
            }
            if let Some(t) = delta.thinking.as_deref() {
                append_thinking_delta_block(&mut g, t);
            }
        }
        // Live tool-call argument fragments are renderer-only: the shared
        // `blocks` buffer (used for persistence) is populated later by the
        // engine's terminal `ToolUse` event via `record_tool_use_block`,
        // so we deliberately don't write the partial input here.
        let tool_call_delta = delta.tool_call.as_ref().map(|tc| {
            serde_json::json!({
                "id": tc.id,
                "name": tc.name,
                "arguments": tc.arguments,
            })
        });
        // Suppress an empty text_delta emitted purely because the cleaner
        // held everything back this chunk (avoids a no-op UI event).
        let emit_text = match cleaned_text.as_deref() {
            Some("") => None,
            other => other.map(|s| s.to_string()),
        };
        let _ = app.emit(
            "gen://stream",
            serde_json::json!({
                "session_id": &session_id,
                "request_message_id": &request_message_id,
                "text_delta": emit_text,
                "thinking_delta": delta.thinking,
                "tool_call_delta": tool_call_delta,
            }),
        );
    })
}

/// Build the `gen://tool` callback. Mirrors [`stream_text_callback`]:
/// updates the shared block buffer first, then forwards a structured
/// payload to the renderer so the UI can render the tool card inline
/// the moment the engine fires the event.
fn tool_event_callback(
    app: AppHandle,
    session_id: String,
    request_message_id: String,
    blocks: StreamBlocks,
) -> ToolEventCallback {
    Arc::new(move |event| match event {
        MessageEvent::ToolUse { id, tool, input } => {
            if let Ok(mut g) = blocks.lock() {
                record_tool_use_block(&mut g, id.as_str(), tool, input);
            }
            let _ = app.emit(
                "gen://tool",
                serde_json::json!({
                    "session_id": &session_id,
                    "request_message_id": &request_message_id,
                    "type": "tool_use",
                    "id": id.as_str(),
                    "tool": tool,
                    "input": input,
                }),
            );
        }
        MessageEvent::ToolResult {
            id,
            tool,
            output,
            is_error,
        } => {
            if let Ok(mut g) = blocks.lock() {
                record_tool_result_block(&mut g, id.as_str(), output, *is_error);
            }
            let _ = app.emit(
                "gen://tool",
                serde_json::json!({
                    "session_id": &session_id,
                    "request_message_id": &request_message_id,
                    "type": "tool_result",
                    "id": id.as_str(),
                    "tool": tool,
                    "output": output,
                    "is_error": is_error,
                }),
            );
        }
        // Other variants (Assistant text, User, Progress, CompactBoundary)
        // aren't structural tool events - ignore them here.
        _ => {}
    })
}

// ????????? Settings ?????????

#[tauri::command]
fn get_settings(state: tauri::State<Arc<AppState>>) -> Result<settings::Settings, AppError> {
    let conn = state.conn()?;
    settings::read(&conn)
}

#[tauri::command]
fn update_settings(
    state: tauri::State<Arc<AppState>>,
    patch: settings::SettingsPatch,
) -> Result<settings::Settings, AppError> {
    let conn = state.conn()?;
    settings::apply_patch(&conn, patch)
}

#[tauri::command]
fn get_llm_model_catalog(
    state: tauri::State<Arc<AppState>>,
) -> Result<llm_catalog::LlmModelCatalogDto, AppError> {
    let conn = state.conn()?;
    llm_catalog::fetch_for_frontend(&conn)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchProviderModelsArgs {
    sdk: String,
    endpoint: String,
    #[serde(default)]
    api_key: String,
}

/// Pull the live model catalog a provider advertises via its `/models`
/// endpoint so the settings dialog can browse and import models.
#[tauri::command]
async fn fetch_provider_models(
    args: FetchProviderModelsArgs,
) -> Result<Vec<String>, AppError> {
    crate::ai::providers::model_list::fetch_models(&args.sdk, &args.endpoint, &args.api_key).await
}

// ????????? App info ?????????

#[derive(Debug, Serialize)]
struct AppInfo {
    version: String,
    data_dir: String,
    db_path: String,
    sessions_dir: String,
}

#[tauri::command]
fn get_app_info(app: AppHandle) -> Result<AppInfo, AppError> {
    let data_dir = paths::root_dir(&app)?;
    let db_path = paths::db_path(&app)?;
    let sessions_dir = paths::sessions_dir(&app)?;
    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: data_dir.to_string_lossy().into_owned(),
        db_path: db_path.to_string_lossy().into_owned(),
        sessions_dir: sessions_dir.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
fn open_path(path: String) -> Result<(), AppError> {
    let p = std::path::Path::new(&path);
    if !p.exists() {
        return Err(AppError::NotFound(format!("path does not exist: {path}")));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer").arg(&path).spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&path).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(&path).spawn()?;
    }
    Ok(())
}

#[tauri::command]
fn toggle_devtools(app: AppHandle) -> Result<(), AppError> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    #[cfg(any(debug_assertions, feature = "devtools"))]
    {
        #[cfg(not(target_os = "windows"))]
        {
            if window.is_devtools_open() {
                window.close_devtools();
                return Ok(());
            }
        }
        window.open_devtools();
    }
    Ok(())
}

// ????????? Sessions ?????????

#[derive(Debug, Deserialize)]
struct CreateSessionArgs {
    title: Option<String>,
    model: Option<String>,
}

#[tauri::command]
fn list_sessions(
    state: tauri::State<Arc<AppState>>,
) -> Result<Vec<session::SessionSummary>, AppError> {
    let conn = state.conn()?;
    session::list(&conn)
}

#[tauri::command]
fn search_sessions(
    state: tauri::State<Arc<AppState>>,
    query: String,
    limit: i64,
) -> Result<Vec<session::SessionSearchResult>, AppError> {
    let conn = state.conn()?;
    session::search(&conn, &query, limit)
}

#[tauri::command]
fn create_session(
    state: tauri::State<Arc<AppState>>,
    args: CreateSessionArgs,
) -> Result<session::Session, AppError> {
    let conn = state.conn()?;
    let mut sess = session::create(&conn, args.title, args.model)?;

    // Auto-apply the active model and its catalog context-window when no
    // explicit model was provided.  Mirrors the behaviour of `set_session_model`
    // so users never have to re-select the same model just to initialise stats.
    if sess.model.is_none() {
        if let Ok(s) = settings::read(&conn) {
            let model = s.model.trim().to_string();
            if !model.is_empty() {
                let sdk = s
                    .model_services
                    .iter()
                    .find(|p| p.id == s.active_provider_id)
                    .map(|p| p.sdk.as_str())
                    .unwrap_or("");
                let cw = llm_catalog::lookup_context_window(
                    &conn,
                    &s.active_provider_id,
                    sdk,
                    &model,
                )
                .ok()
                .flatten();
                let _ = session::set_model_and_context(
                    &conn,
                    &sess.id,
                    Some(model.as_str()),
                    cw,
                );
                sess.model = Some(model);
                sess.context_window = cw;
            }
        }
    }

    Ok(sess)
}

#[tauri::command]
fn rename_session(
    state: tauri::State<Arc<AppState>>,
    id: String,
    title: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::rename(&conn, &id, &title)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateSessionConfigArgs {
    id: String,
    system_prompt: String,
    history_turns: i64,
    llm_params: settings::ModelParamSettings,
}

#[tauri::command]
fn update_session_config(
    state: tauri::State<Arc<AppState>>,
    args: UpdateSessionConfigArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::update_config(
        &conn,
        &args.id,
        &args.system_prompt,
        args.history_turns,
        &args.llm_params,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSessionModelArgs {
    id: String,
    model: String,
    context_window: Option<i64>,
}

#[tauri::command]
fn set_session_model(
    state: tauri::State<Arc<AppState>>,
    args: SetSessionModelArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let mut cw = args.context_window;
    if cw.is_none() {
        let s = settings::read(&conn)?;
        let sdk = s
            .model_services
            .iter()
            .find(|p| p.id == s.active_provider_id)
            .map(|p| p.sdk.as_str())
            .unwrap_or("");
        cw = llm_catalog::lookup_context_window(
            &conn,
            &s.active_provider_id,
            sdk,
            &args.model,
        )?;
    }
    session::set_model_and_context(
        &conn,
        &args.id,
        Some(args.model.as_str()),
        cw,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSessionAgentTypeArgs {
    id: String,
    agent_type: String,
}

#[tauri::command]
fn set_session_agent_type(
    state: tauri::State<Arc<AppState>>,
    args: SetSessionAgentTypeArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::set_agent_type(&conn, &args.id, &args.agent_type)
}

#[tauri::command]
fn delete_session(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> Result<(), AppError> {
    {
        let conn = state.conn()?;
        let scope = crate::data::role_state::resolve_role_state_scope(&conn, &id)?;
        session::delete(&conn, &id)?;
        // Standalone sessions own their scope; project sessions share scope.
        if scope == id {
            let _ = crate::data::role_state::clear_scope(&conn, &scope);
            state.role_states.clear(&scope);
        }
        let _ = crate::data::file_snapshot::clear_session(&conn, &id);
    }
    state.file_snapshots.clear(&id);
    state.token_logger.delete_session_log(&id);
    let dir = paths::sessions_dir(&app)?.join(&id);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}

#[tauri::command]
fn load_session(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> Result<SessionWithMessagesAbs, AppError> {
    let conn = state.conn()?;
    let scope = crate::data::role_state::resolve_role_state_scope(&conn, &id)?;
    // Re-hydrate the in-memory role board so the next role-state run sees the
    // persisted truth and the UI can fetch it via `get_role_states`.
    if let Ok(roles) = crate::data::role_state::latest_roles(&conn, &scope) {
        state.role_states.load(&scope, roles);
    }
    let mut s = session::load_with_messages(&conn, &id)?;
    // Sessions in a project share the project's single agent flow record;
    // surface it as the session's chain so the UI edits/reads one source of
    // truth regardless of which conversation is open.
    if let Some(ref pid) = s.session.project_id {
        if let Ok(proj) = project::get(&conn, pid) {
            s.session.agent_chain = proj.agent_chain;
        }
    }
    Ok(decorate_session(&app, s))
}

/// Return the current character state board for a session as a JSON array of
/// role objects (insertion order preserved).
#[tauri::command]
fn get_role_states(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
) -> Result<Vec<serde_json::Value>, AppError> {
    let conn = state.conn()?;
    let scope = crate::data::role_state::resolve_role_state_scope(&conn, &session_id)?;
    // Prefer the live in-memory board; fall back to the persisted snapshot
    // when the scope hasn't been touched this process lifetime.
    let live = state.role_states.snapshot(&scope);
    if !live.is_empty() {
        return Ok(live);
    }
    let roles = crate::data::role_state::latest_roles(&conn, &scope)?;
    state.role_states.load(&scope, roles.clone());
    Ok(roles)
}

// ????????? Projects ?????????

#[derive(Debug, Deserialize)]
struct CreateProjectArgs {
    name: String,
    path: Option<String>,
}

#[tauri::command]
fn list_projects(
    state: tauri::State<Arc<AppState>>,
) -> Result<Vec<project::Project>, AppError> {
    let conn = state.conn()?;
    project::list(&conn)
}

#[tauri::command]
fn create_project(
    state: tauri::State<Arc<AppState>>,
    args: CreateProjectArgs,
) -> Result<project::Project, AppError> {
    let conn = state.conn()?;
    project::create(&conn, &args.name, args.path.as_deref())
}

#[tauri::command]
fn rename_project(
    state: tauri::State<Arc<AppState>>,
    id: String,
    name: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    project::rename(&conn, &id, &name)
}

#[tauri::command]
fn update_project_path(
    state: tauri::State<Arc<AppState>>,
    id: String,
    path: Option<String>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    project::set_path(&conn, &id, path.as_deref())
}

#[tauri::command]
fn delete_project(
    state: tauri::State<Arc<AppState>>,
    id: String,
) -> Result<(), AppError> {
    {
        let conn = state.conn()?;
        let _ = crate::data::role_state::clear_scope(&conn, &id);
        project::delete(&conn, &id)?;
    }
    state.role_states.clear(&id);
    Ok(())
}

#[tauri::command]
fn reorder_projects(
    state: tauri::State<Arc<AppState>>,
    ordered_ids: Vec<String>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    project::reorder(&conn, &ordered_ids)
}

#[tauri::command]
fn assign_session_to_project(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    project_id: Option<String>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    if let Some(ref pid) = project_id {
        let _ = crate::data::role_state::reassign_session_scope(&conn, &session_id, pid);
        // Re-hydrate project scope in memory if we already had session-scoped data.
        if let Ok(roles) = crate::data::role_state::latest_roles(&conn, pid) {
            state.role_states.load(pid, roles);
        }
    }
    project::assign_session(&conn, &session_id, project_id.as_deref())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateProjectConfigArgs {
    id: String,
    system_prompt: String,
    history_turns: i64,
    llm_params: settings::ModelParamSettings,
    context_window: Option<i64>,
}

#[tauri::command]
fn update_project_config(
    state: tauri::State<Arc<AppState>>,
    args: UpdateProjectConfigArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    project::update_config(
        &conn,
        &args.id,
        &args.system_prompt,
        args.history_turns,
        &args.llm_params,
        args.context_window,
    )
}

// ????????? Messages ?????????

#[tauri::command]
fn update_message_text(
    state: tauri::State<Arc<AppState>>,
    id: String,
    text: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::update_message_text(&conn, &id, &text)
}

#[tauri::command]
fn update_message_images(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
    image_ids: Vec<String>,
) -> Result<MessageAbs, AppError> {
    let removed = {
        let conn = state.conn()?;
        session::update_message_input_images(&conn, &id, &image_ids)?
    };
    for (rel, thumb) in removed {
        if let Ok(abs) = paths::abs_from_rel(&app, &rel) {
            let _ = std::fs::remove_file(&abs);
        }
        if let Some(t) = thumb {
            if let Ok(abs) = paths::abs_from_rel(&app, &t) {
                let _ = std::fs::remove_file(&abs);
            }
        }
    }
    let conn = state.conn()?;
    let m = reload_message(&conn, &id)?;
    Ok(decorate_message(&app, m))
}

#[tauri::command]
fn delete_message(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> Result<(), AppError> {
    // Capture the owning session before the row is gone so we can roll the
    // character state board back to whatever it was before this message.
    let session_id = {
        let conn = state.conn()?;
        reload_message(&conn, &id).ok().map(|m| m.session_id)
    };

    if let Some(ref sid) = session_id {
        let conn = state.conn()?;
        state
            .token_logger
            .rollback_jsonl_from_message(&conn, sid, &id);
    }

    let paths = {
        let conn = state.conn()?;
        session::delete_message(&conn, &id)?
    };
    for (rel, thumb) in paths {
        if let Ok(abs) = paths::abs_from_rel(&app, &rel) {
            let _ = std::fs::remove_file(&abs);
        }
        if let Some(t) = thumb {
            if let Ok(abs) = paths::abs_from_rel(&app, &t) {
                let _ = std::fs::remove_file(&abs);
            }
        }
    }

    if let Some(sid) = session_id {
        let conn = state.conn()?;
        let scope = crate::data::role_state::resolve_role_state_scope(&conn, &sid)?;
        if let Ok(roles) = crate::data::role_state::rollback_from_message(&conn, &scope, &id) {
            state.role_states.load(&scope, roles);
            emit_role_state_reset(&app, &scope, &sid);
        }
        // Roll the workspace back: restore / delete every file this message (and
        // any later ones) created, updated or removed.
        if let Ok(restores) = crate::data::file_snapshot::rollback_from_message(&conn, &sid, &id) {
            for r in &restores {
                apply_file_restore(r);
            }
        }
    }
    Ok(())
}

/// Apply a single file-snapshot rollback action to disk: delete a file that
/// was created within the rolled-back range, or rewrite a file with its
/// captured pre-image.
fn apply_file_restore(restore: &crate::data::file_snapshot::FileRestore) {
    if restore.delete {
        let _ = std::fs::remove_file(&restore.path);
        return;
    }
    if let Some(content) = &restore.content {
        let encoding = restore
            .encoding
            .as_deref()
            .map(TextEncoding::parse_label)
            .unwrap_or(TextEncoding::Utf8);
        let _ = write_text_file_labeled(
            &restore.path,
            content,
            Some(encoding.label()),
            Some(restore.had_bom),
        );
    }
}

/// Tell the UI to discard its in-memory role board for a scope and re-fetch
/// the persisted truth (used after a rollback / message deletion).
fn emit_role_state_reset(app: &AppHandle, scope_id: &str, session_id: &str) {
    let _ = app.emit(
        "role-state://reset",
        serde_json::json!({
            "scope_id": scope_id,
            "session_id": session_id,
        }),
    );
}

#[tauri::command]
async fn quote_message_as_attachments(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    session_id: String,
    message_id: String,
) -> Result<Vec<images::AttachmentDraft>, AppError> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        const MAX_ATTACH: usize = 8;
        let conn = state.conn()?;
        let mut msg = reload_message(&conn, &message_id)?;
        if msg.session_id != session_id {
            return Err(AppError::Invalid("message not in session".into()));
        }
        msg.images.sort_by_key(|i| i.ord);
        let mut out = Vec::new();
        for img in msg.images {
            if matches!(img.role.as_str(), "input" | "output" | "edited") {
                let d = images::clone_image_as_draft(&app, &conn, &session_id, &img.id)?;
                out.push(d);
                if out.len() >= MAX_ATTACH {
                    break;
                }
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?
}

// ????????? Attachments ?????????

#[tauri::command]
async fn add_attachment_from_path(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    session_id: String,
    path: String,
) -> Result<images::AttachmentDraft, AppError> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = state.conn()?;
        images::save_path_as_attachment(&app, &conn, &session_id, std::path::Path::new(&path))
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?
}

#[derive(Debug, Deserialize)]
struct AttachBytesArgs {
    session_id: String,
    name: Option<String>,
    bytes: Vec<u8>,
}

#[tauri::command]
async fn add_attachment_from_bytes(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    args: AttachBytesArgs,
) -> Result<images::AttachmentDraft, AppError> {
    let state = state.inner().clone();
    tokio::task::spawn_blocking(move || {
        let conn = state.conn()?;
        images::save_bytes_as_attachment(
            &app,
            &conn,
            &args.session_id,
            args.name.as_deref(),
            &args.bytes,
        )
    })
    .await
    .map_err(|e| AppError::Other(e.to_string()))?
}

#[tauri::command]
fn remove_attachment_draft(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    image_id: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let img = session::get_image(&conn, &image_id)?;
    if let Ok(abs) = paths::abs_from_rel(&app, &img.rel_path) {
        let _ = std::fs::remove_file(&abs);
    }
    if let Some(thumb) = &img.thumb_rel_path {
        if let Ok(abs) = paths::abs_from_rel(&app, thumb) {
            let _ = std::fs::remove_file(&abs);
        }
    }
    let conn = state.conn()?;
    conn.execute(
        "DELETE FROM message_images WHERE id=?1 AND message_id IS NULL",
        rusqlite::params![image_id],
    )?;
    Ok(())
}

// ????????? Image asset URL helpers ?????????

#[tauri::command]
fn get_image_abs_path(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    image_id: String,
) -> Result<String, AppError> {
    let conn = state.conn()?;
    let img = session::get_image(&conn, &image_id)?;
    let abs = paths::abs_from_rel(&app, &img.rel_path)?;
    Ok(abs.to_string_lossy().to_string())
}

// ????????? Generate ?????????

#[tauri::command]
fn cancel_generation(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
) -> Result<(), AppError> {
    let guard = generation_abort_lock(&state)?;
    if let Some(handle) = guard.get(&session_id) {
        handle.abort();
    }
    Ok(())
}

// ????????? Agent task commands ?????????

#[tauri::command]
fn list_agent_tasks(state: tauri::State<Arc<AppState>>) -> Result<Vec<Task>, AppError> {
    Ok(state.task_store.list())
}

#[tauri::command]
fn cancel_agent_task(
    state: tauri::State<Arc<AppState>>,
    task_id: String,
) -> Result<(), AppError> {
    let id = agent::TaskId(task_id);
    state.task_store.set_state(&id, TaskState::Killed);
    // After state transitions, surface the kill to the main loop as a
    // hidden `<task-notification>` for the next request.
    if let Some(slot) = state.task_store.get(&id) {
        if let Ok(t) = slot.lock() {
            if let Some(note) = agent::TaskNotification::from_task(&t) {
                state
                    .notifications
                    .push(agent::Attachment::for_main(agent::AttachmentKind::TaskNotification(note)));
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct AgentSummary {
    agent_type: String,
    when_to_use: String,
    background: bool,
    tools: Vec<String>,
    disallowed_tools: Vec<String>,
}

#[derive(Debug, Serialize)]
struct UserContextSummary {
    file_count: usize,
    rendered_chars: usize,
    files: Vec<UserContextFile>,
}

#[derive(Debug, Serialize)]
struct UserContextFile {
    ty: String,
    path: String,
    conditional: bool,
    path_globs: Option<Vec<String>>,
}

#[tauri::command]
fn refresh_user_context(
    state: tauri::State<Arc<AppState>>,
) -> Result<UserContextSummary, AppError> {
    use crate::ai::agent::memory::UserContextLoader;
    state.user_context.invalidate();
    let ctx = state.user_context.load()?;
    Ok(UserContextSummary {
        file_count: ctx.memory_files.len(),
        rendered_chars: ctx.rendered.chars().count(),
        files: ctx
            .memory_files
            .iter()
            .map(|mf| UserContextFile {
                ty: format!("{:?}", mf.ty).to_lowercase(),
                path: mf.path.to_string_lossy().into_owned(),
                conditional: mf.conditional,
                path_globs: mf.path_globs.clone(),
            })
            .collect(),
    })
}

#[tauri::command]
fn set_mcp_servers(
    state: tauri::State<Arc<AppState>>,
    servers: Vec<String>,
) -> Result<(), AppError> {
    state.mcp.set(servers);
    Ok(())
}

#[derive(Debug, Serialize)]
struct SessionMemoryInfo {
    session_id: String,
    summary_path: String,
    total_tokens: i64,
}

#[tauri::command]
fn list_agent_tools(state: tauri::State<Arc<AppState>>) -> Result<Vec<String>, AppError> {
    let mut names: Vec<String> = state
        .tools
        .all()
        .into_iter()
        .map(|t| t.spec().name.clone())
        .collect();
    names.sort();
    Ok(names)
}

#[tauri::command]
fn extract_session_memory(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    session_id: String,
) -> Result<SessionMemoryInfo, AppError> {
    let dir = paths::session_dir(&app, &session_id)?;

    // Use the most recent completed task for this agent_type/session as
    // the source. If nothing matches we still write the default template.
    let latest_task = state
        .task_store
        .list()
        .into_iter()
        .filter(|t| !matches!(t.state, TaskState::Pending | TaskState::Running))
        .max_by_key(|t| t.ended_at_ms.unwrap_or(t.started_at_ms));

    let sm = state
        .session_memory
        .extract_now(&session_id, &dir, latest_task.as_ref())?;

    Ok(SessionMemoryInfo {
        session_id: sm.session_id,
        summary_path: sm.summary_path.to_string_lossy().into_owned(),
        total_tokens: sm.last_usage.total_tokens.unwrap_or(0),
    })
}

#[derive(Debug, Deserialize)]
struct TokenUsageSummaryArgs {
    #[serde(default)]
    from_ms: Option<i64>,
    #[serde(default)]
    to_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ListTokenUsageEventsArgs {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    event_kind: Option<String>,
    #[serde(default)]
    from_ms: Option<i64>,
    #[serde(default)]
    to_ms: Option<i64>,
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
}

#[tauri::command]
fn get_token_usage_summary(
    state: tauri::State<Arc<AppState>>,
    args: TokenUsageSummaryArgs,
) -> Result<crate::data::token_log::TokenUsageSummary, AppError> {
    let conn = state.conn()?;
    crate::data::token_log::query_summary(&conn, args.from_ms, args.to_ms)
}

#[tauri::command]
fn list_token_usage_events(
    state: tauri::State<Arc<AppState>>,
    args: ListTokenUsageEventsArgs,
) -> Result<Vec<crate::data::token_log::TokenUsageEvent>, AppError> {
    let conn = state.conn()?;
    crate::data::token_log::list_events(
        &conn,
        &crate::data::token_log::TokenUsageListFilter {
            session_id: args.session_id,
            model: args.model,
            event_kind: args.event_kind,
            from_ms: args.from_ms,
            to_ms: args.to_ms,
            limit: args.limit.unwrap_or(100),
            offset: args.offset.unwrap_or(0),
        },
    )
}

#[tauri::command]
fn list_agents(state: tauri::State<Arc<AppState>>) -> Result<Vec<AgentSummary>, AppError> {
    let mut out: Vec<AgentSummary> = state
        .agent_registry
        .active()
        .into_values()
        .map(|d| AgentSummary {
            agent_type: d.agent_type,
            when_to_use: d.when_to_use,
            background: d.background,
            tools: d.tools,
            disallowed_tools: d.disallowed_tools,
        })
        .collect();
    out.sort_by(|a, b| a.agent_type.cmp(&b.agent_type));
    Ok(out)
}

/// Full resolved configuration for one agent type, used to pre-fill the
/// per-node config editor with the agent's default values. Resolves built-ins
/// from the registry and custom agents from the DB.
#[derive(Debug, Serialize)]
struct AgentDefinitionInfo {
    agent_type: String,
    when_to_use: String,
    system_prompt: String,
    model: Option<String>,
    tools: Vec<String>,
    background: bool,
    passthrough_output: bool,
}

#[tauri::command]
fn get_agent_definition(
    state: tauri::State<Arc<AppState>>,
    agent_type: String,
) -> Result<AgentDefinitionInfo, AppError> {
    let d = resolve_generation_definition(&state, &agent_type)?;
    Ok(AgentDefinitionInfo {
        agent_type: d.agent_type,
        when_to_use: d.when_to_use,
        system_prompt: d.system_prompt,
        model: d.model,
        tools: d.tools,
        background: d.background,
        passthrough_output: d.passthrough_output,
    })
}

#[tauri::command]
fn list_custom_agents(
    state: tauri::State<Arc<AppState>>,
) -> Result<Vec<custom_agents::CustomAgent>, AppError> {
    let conn = state.conn()?;
    custom_agents::list(&conn)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateCustomAgentArgs {
    name: String,
    #[serde(default)]
    when_to_use: String,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Vec<String>,
}

#[tauri::command]
fn create_custom_agent(
    state: tauri::State<Arc<AppState>>,
    args: CreateCustomAgentArgs,
) -> Result<custom_agents::CustomAgent, AppError> {
    let conn = state.conn()?;
    custom_agents::create(
        &conn,
        &args.name,
        &args.when_to_use,
        &args.system_prompt,
        args.model.as_deref(),
        &args.tools,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UpdateCustomAgentArgs {
    agent_type: String,
    name: String,
    #[serde(default)]
    when_to_use: String,
    #[serde(default)]
    system_prompt: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    tools: Vec<String>,
}

#[tauri::command]
fn update_custom_agent(
    state: tauri::State<Arc<AppState>>,
    args: UpdateCustomAgentArgs,
) -> Result<custom_agents::CustomAgent, AppError> {
    let conn = state.conn()?;
    custom_agents::update(
        &conn,
        &args.agent_type,
        &args.name,
        &args.when_to_use,
        &args.system_prompt,
        args.model.as_deref(),
        &args.tools,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeleteCustomAgentArgs {
    agent_type: String,
}

#[tauri::command]
fn delete_custom_agent(
    state: tauri::State<Arc<AppState>>,
    args: DeleteCustomAgentArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    custom_agents::delete(&conn, &args.agent_type)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetSessionAgentChainArgs {
    id: String,
    chain: Vec<session::ChainNode>,
}

#[tauri::command]
fn set_session_agent_chain(
    state: tauri::State<Arc<AppState>>,
    args: SetSessionAgentChainArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::set_agent_chain(&conn, &args.id, &args.chain)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetProjectAgentChainArgs {
    id: String,
    chain: Vec<session::ChainNode>,
}

#[tauri::command]
fn set_project_agent_chain(
    state: tauri::State<Arc<AppState>>,
    args: SetProjectAgentChainArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    project::set_agent_chain(&conn, &args.id, &args.chain)
}

#[derive(Debug, Deserialize)]
struct GenerateReq {
    session_id: String,
    prompt: String,
    attachment_ids: Vec<String>,
    aspect_ratio: String,
    image_size: String,
    #[serde(default)]
    thinking_enabled: Option<bool>,
    #[serde(default)]
    thinking_effort: Option<String>,
}

#[derive(Debug, Serialize)]
struct GenerateResult {
    user_message: MessageAbs,
    assistant_message: MessageAbs,
}

/// Dedupe multimodal duplicates, persist assistant row + output images, return API DTO.
#[allow(clippy::too_many_arguments)]
fn finalize_generate_assistant_message(
    app: &AppHandle,
    conn: &db::DbConn,
    session_id: &str,
    user_message_id: &str,
    params: &parameters::GenerationParameters,
    mut resp: chat::GenerateResponse,
    mut blocks: Vec<serde_json::Value>,
    role_states: &RoleStateStore,
    file_snapshots: &FileSnapshotStore,
    token_logger: &token_log::TokenUsageLogger,
    agent_type: &str,
    model: &str,
    provider: &str,
) -> AppResult<GenerateResult> {
    use crate::ai::stream_split::strip_leaked_host_tool_log;

    resp.images = chat::dedupe_image_results(resp.images);

    // #region agent log
    let raw_resp_has_marker = resp.text.as_deref().map(agent_dbg_has_marker).unwrap_or(false);
    let raw_block_has_marker = blocks.iter().any(|b| {
        b.get("type").and_then(|v| v.as_str()) == Some("text")
            && b.get("content")
                .and_then(|c| c.as_str())
                .map(agent_dbg_has_marker)
                .unwrap_or(false)
    });
    // #endregion

    // Belt-and-suspenders: scrub any leaked host tool-log lines out of the
    // persisted text blocks and the final reply before they ever hit the DB.
    for b in &mut blocks {
        if b.get("type").and_then(|v| v.as_str()) == Some("text") {
            if let Some(t) = b.get("content").and_then(|c| c.as_str()) {
                let cleaned = strip_leaked_host_tool_log(t);
                if let Some(obj) = b.as_object_mut() {
                    obj.insert("content".into(), serde_json::Value::String(cleaned));
                }
            }
        }
    }
    if let Some(t) = resp.text.as_deref() {
        resp.text = Some(strip_leaked_host_tool_log(t));
    }

    let block_text = concat_block_text(&blocks, "text");
    let block_thinking = concat_block_text(&blocks, "thinking");
    // #region agent log
    agent_dbg(
        "app/mod.rs:finalize_generate_assistant_message",
        "persisting assistant message",
        "E",
        serde_json::json!({
            "session_id": session_id,
            "user_message_id": user_message_id,
            "raw_resp_has_marker": raw_resp_has_marker,
            "raw_block_has_marker": raw_block_has_marker,
            "resp_text_has_marker_after_clean": resp.text.as_deref().map(agent_dbg_has_marker).unwrap_or(false),
            "block_text_has_marker_after_clean": agent_dbg_has_marker(&block_text),
            "block_text_len": block_text.len(),
        }),
    );
    // #endregion
    if resp
        .text
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
        && !block_text.trim().is_empty()
    {
        resp.text = Some(block_text.trim().to_string());
    }
    if resp
        .thinking_content
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
        && !block_thinking.trim().is_empty()
    {
        resp.thinking_content = Some(block_thinking.trim().to_string());
    } else if block_thinking.len()
        > resp
            .thinking_content
            .as_ref()
            .map(|s| s.len())
            .unwrap_or(0)
    {
        resp.thinking_content = Some(block_thinking.trim().to_string());
    }
    let mut assistant_params =
        params.to_assistant_message_params(&resp.usage, resp.thinking_content.as_deref());
    if !blocks.is_empty() {
        // Persist the structured timeline alongside blocks so future turns
        // replay tool history in native call/response form instead of a
        // leak-prone plain-text transcript.
        let timeline = crate::ai::block_timeline::restore_timeline_from_blocks(&blocks);
        if let Some(obj) = assistant_params.as_object_mut() {
            if !timeline.is_empty() {
                if let Ok(tv) = serde_json::to_value(&timeline) {
                    obj.insert("timeline".into(), tv);
                }
            }
            obj.insert("blocks".into(), serde_json::Value::Array(blocks));
        }
    }
    let assistant_params_json = assistant_params.to_string();
    let assistant = session::insert_message(
        conn,
        session_id,
        "assistant",
        resp.text.as_deref(),
        Some(assistant_params_json.as_str()),
    )?;
    for (i, img) in resp.images.iter().enumerate() {
        images::write_output_image(
            app,
            conn,
            session_id,
            &assistant.id,
            &img.bytes,
            &img.mime,
            i as i64,
        )?;
    }
    // Snapshot the character state board against this assistant message so it
    // can be re-hydrated on session open and rolled back on delete/regenerate.
    let scope_id = crate::data::role_state::resolve_role_state_scope(conn, session_id)?;
    let roles = role_states.snapshot(&scope_id);
    if !roles.is_empty() {
        let _ = crate::data::role_state::save_snapshot(
            conn,
            &scope_id,
            session_id,
            &assistant.id,
            &roles,
        );
    }

    // Bind any file mutations captured during this generation to this message
    // so they can be rolled back when it is deleted / regenerated.
    let file_changes = file_snapshots.take(session_id);
    if !file_changes.is_empty() {
        let _ = crate::data::file_snapshot::save_changes(
            conn,
            session_id,
            &assistant.id,
            &file_changes,
        );
    }

    token_logger.log_turn_summary(token_log::TurnSummaryLog {
        ctx: token_log::LogContext {
            session_id: Some(session_id.to_string()),
            correlation_id: Some(user_message_id.to_string()),
            agent_id: None,
            agent_type: Some(agent_type.to_string()),
        },
        message_id: assistant.id.clone(),
        model: model.to_string(),
        provider: provider.to_string(),
        usage: resp.usage.clone(),
    });

    let user_full = reload_message(conn, user_message_id)?;
    let assistant_full = reload_message(conn, &assistant.id)?;
    session::recompute_context_window_used(conn, session_id)?;
    Ok(GenerateResult {
        user_message: decorate_message(app, user_full),
        assistant_message: decorate_message(app, assistant_full),
    })
}

#[tauri::command]
async fn generate_image(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    req: GenerateReq,
) -> Result<GenerateResult, AppError> {
    // 1) gather settings + attachment bytes + history synchronously
    let (chat_request, params, attachment_image_ids, generation_agent, project_cwd, agent_chain, settings_snapshot, session_prompt) = {
        let conn = state.conn()?;
        let s = settings::read(&conn)?;
        let session_config = session::get(&conn, &req.session_id)?;
        let generation_agent =
            session::generation_agent_definition_key(&session_config.agent_type);
        let agent_chain = effective_agent_chain(&conn, &session_config);
        let project_cwd = session_project_cwd(&conn, &req.session_id);
        let eff = effective_session_params(&conn, &session_config);
        let session_prompt = eff.system_prompt;
        let history_turns = eff.history_turns;
        // Thinking is now driven by the composer (per-request) rather than
        // session/project config: override any stored llm_params thinking values.
        let mut model_params = eff.llm_params;
        model_params.thinking_enabled = req.thinking_enabled;
        model_params.thinking_effort = req
            .thinking_effort
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let mut atts: Vec<chat::AttachmentBytes> = Vec::new();
        let mut ids: Vec<String> = Vec::new();
        for id in &req.attachment_ids {
            let img = session::get_image(&conn, id)?;
            let bytes = images::read_image_bytes(&app, &img)?;
            atts.push(chat::AttachmentBytes {
                bytes,
                mime: img.mime.clone(),
            });
            ids.push(img.id.clone());
        }
        let params = parameters::factory().build(
            req.aspect_ratio.clone(),
            req.image_size.clone(),
            model_params,
        );
        let hist = build_history(
            &app,
            &conn,
            &req.session_id,
            None,
            history_turns.max(0) as usize,
        )?;
        let chat_request = router::build_chat_request(
            &s,
            req.prompt.clone(),
            atts,
            session_prompt.clone(),
            hist,
            params.clone(),
        )?;
        (chat_request, params, ids, generation_agent, project_cwd, agent_chain, s, session_prompt)
    };
    let params_json = params.to_message_params_json().to_string();

    // 2) insert user message + bind input attachments
    let user_msg = {
        let conn = state.conn()?;
        let m = session::insert_message(
            &conn,
            &req.session_id,
            "user",
            Some(req.prompt.as_str()),
            Some(params_json.as_str()),
        )?;
        session::bind_images_to_message(&conn, &m.id, &attachment_image_ids)?;
        m
    };

    // ensure session title reflects first prompt
    {
        let conn = state.conn()?;
        if session_title_is_default(&conn, &req.session_id)? {
            match settings::quick_model_target(&settings_snapshot) {
                Some((provider, model_id)) => {
                    // Generate a concise title with the configured quick model
                    // off the request path so it never delays the first token.
                    let provider_cfg = chat::ProviderConfig {
                        id: provider.id.clone(),
                        name: provider.name.clone(),
                        sdk: crate::ai::providers::normalize_sdk(&provider.sdk),
                        endpoint: provider.endpoint.clone(),
                        api_key: provider.api_key.clone(),
                    };
                    tokio::spawn(generate_title_with_quick_model(
                        app.clone(),
                        state.inner().clone(),
                        req.session_id.clone(),
                        req.prompt.clone(),
                        provider_cfg,
                        model_id,
                    ));
                }
                None => {
                    update_session_title_if_default(&conn, &req.session_id, &req.prompt)?;
                }
            }
        }
    }

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "request",
            "session_id": &req.session_id,
            "message_id": &user_msg.id,
        }),
    );

    // 3) call the unified chat router
    let stream_blocks = new_stream_blocks();
    let on_text_delta = stream_text_callback(
        app.clone(),
        req.session_id.clone(),
        user_msg.id.clone(),
        stream_blocks.clone(),
    );
    let on_tool_event = tool_event_callback(
        app.clone(),
        req.session_id.clone(),
        user_msg.id.clone(),
        stream_blocks.clone(),
    );
    let log_model = chat_request.model.clone();
    let log_provider = chat_request.provider.id.clone();
    let result = match agent_chain.as_ref().filter(|c| !c.is_empty()) {
        Some(chain) => {
            run_agent_chain(
                &state,
                &app,
                &req.session_id,
                &user_msg.id,
                chain,
                generation_agent,
                &req.prompt,
                chat_request,
                &settings_snapshot,
                &session_prompt,
                &params,
                project_cwd,
                &stream_blocks,
                on_text_delta,
                on_tool_event,
            )
            .await
        }
        None => {
            run_cancellable_generation(
                &state,
                &req.session_id,
                generation_agent,
                req.prompt.clone(),
                chat_request,
                Some(on_text_delta),
                Some(on_tool_event),
                project_cwd,
                None,
                Some(&user_msg.id),
            )
            .await
        }
    };

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "response",
            "session_id": &req.session_id,
        }),
    );

    // 4) write assistant message
    match result {
        Ok(resp) => {
            maybe_extract_session_memory(&state, &app, &req.session_id, &resp.usage);
            let blocks = snapshot_stream_blocks(&stream_blocks);
            let conn = state.conn()?;
            finalize_generate_assistant_message(
                &app,
                &conn,
                &req.session_id,
                &user_msg.id,
                &params,
                resp,
                blocks,
                &state.role_states,
                &state.file_snapshots,
                &state.token_logger,
                generation_agent,
                &log_model,
                &log_provider,
            )
        }
        Err(AppError::Canceled) => Err(AppError::Canceled),
        Err(e) => {
            let conn = state.conn()?;
            let blocks = snapshot_stream_blocks(&stream_blocks);
            persist_streamed_assistant_snapshot(
                &conn,
                &req.session_id,
                &blocks,
                None,
                None,
                serde_json::json!({ "partial_before_error": true }),
                &state.file_snapshots,
            )?;
            let msg_text = format!("{}", e);
            let err_msg = session::insert_message(
                &conn,
                &req.session_id,
                "error",
                Some(&msg_text),
                Some(params_json.as_str()),
            )?;
            let user_full = reload_message(&conn, &user_msg.id)?;
            Ok(GenerateResult {
                user_message: decorate_message(&app, user_full),
                assistant_message: decorate_message(&app, err_msg),
            })
        }
    }
}

/// Save partial assistant content when the user interrupts generation.
///
/// The frontend accumulates streaming text in a temporary in-memory message.
/// After cancellation, it calls this command to persist whatever was generated
/// before the interrupt so the content isn't lost on session reload.
#[tauri::command]
fn save_cancelled_message(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    text: String,
    thinking: String,
    blocks: Option<serde_json::Value>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let block_vec: Vec<serde_json::Value> = blocks
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|a| a.clone())
        .unwrap_or_default();
    persist_streamed_assistant_snapshot(
        &conn,
        &session_id,
        &block_vec,
        Some(text.as_str()),
        Some(thinking.as_str()),
        serde_json::json!({ "cancelled": true }),
        &state.file_snapshots,
    )
}

#[derive(Debug, Deserialize)]
struct RegenerateReq {
    session_id: String,
    user_message_id: String,
    aspect_ratio: String,
    image_size: String,
    #[serde(default)]
    thinking_enabled: Option<bool>,
    #[serde(default)]
    thinking_effort: Option<String>,
}

#[tauri::command]
async fn regenerate_image(
    state: tauri::State<'_, Arc<AppState>>,
    app: AppHandle,
    req: RegenerateReq,
) -> Result<GenerateResult, AppError> {
    let user_msg_existing = {
        let conn = state.conn()?;
        let m = reload_message(&conn, &req.user_message_id)?;
        if m.session_id != req.session_id {
            return Err(AppError::Invalid(
                "user_message_id does not belong to session".into(),
            ));
        }
        if m.role != "user" {
            return Err(AppError::Invalid("message must be role user".into()));
        }
        m
    };
    let prompt = user_msg_existing
        .text
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::Invalid("user message has no prompt text".into()))?;

    let (chat_request, params, generation_agent, project_cwd, agent_chain, settings_snapshot, session_prompt) = {
        let conn = state.conn()?;
        let s = settings::read(&conn)?;
        let session_config = session::get(&conn, &req.session_id)?;
        let generation_agent =
            session::generation_agent_definition_key(&session_config.agent_type);
        let agent_chain = effective_agent_chain(&conn, &session_config);
        let project_cwd = session_project_cwd(&conn, &req.session_id);
        let eff = effective_session_params(&conn, &session_config);
        let session_prompt = eff.system_prompt;
        let history_turns = eff.history_turns;
        // Thinking is now driven by the composer (per-request) rather than
        // session/project config: override any stored llm_params thinking values.
        let mut model_params = eff.llm_params;
        model_params.thinking_enabled = req.thinking_enabled;
        model_params.thinking_effort = req
            .thinking_effort
            .as_ref()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let mut atts: Vec<chat::AttachmentBytes> = Vec::new();
        let mut input_images: Vec<&session::ImageRef> = user_msg_existing
            .images
            .iter()
            .filter(|i| i.role == "input")
            .collect();
        input_images.sort_by_key(|i| i.ord);
        for img in input_images {
            let bytes = images::read_image_bytes(&app, img)?;
            atts.push(chat::AttachmentBytes {
                bytes,
                mime: img.mime.clone(),
            });
        }
        let params = parameters::factory().build(
            req.aspect_ratio.clone(),
            req.image_size.clone(),
            model_params,
        );
        let params_json = params.to_message_params_json().to_string();
        session::update_message_params(&conn, &req.user_message_id, &params_json)?;
        session::touch(&conn, &req.session_id)?;
        let hist = build_history(
            &app,
            &conn,
            &req.session_id,
            Some(user_msg_existing.created_at),
            history_turns.max(0) as usize,
        )?;
        let chat_request = router::build_chat_request(
            &s,
            prompt.to_string(),
            atts,
            session_prompt.clone(),
            hist,
            params.clone(),
        )?;
        (chat_request, params, generation_agent, project_cwd, agent_chain, s, session_prompt)
    };
    let params_json = params.to_message_params_json().to_string();

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "request",
            "session_id": &req.session_id,
            "message_id": &req.user_message_id,
        }),
    );

    let stream_blocks = new_stream_blocks();
    let on_text_delta = stream_text_callback(
        app.clone(),
        req.session_id.clone(),
        req.user_message_id.clone(),
        stream_blocks.clone(),
    );
    let on_tool_event = tool_event_callback(
        app.clone(),
        req.session_id.clone(),
        req.user_message_id.clone(),
        stream_blocks.clone(),
    );
    let log_model = chat_request.model.clone();
    let log_provider = chat_request.provider.id.clone();
    let result = match agent_chain.as_ref().filter(|c| !c.is_empty()) {
        Some(chain) => {
            run_agent_chain(
                &state,
                &app,
                &req.session_id,
                &req.user_message_id,
                chain,
                generation_agent,
                prompt,
                chat_request,
                &settings_snapshot,
                &session_prompt,
                &params,
                project_cwd,
                &stream_blocks,
                on_text_delta,
                on_tool_event,
            )
            .await
        }
        None => {
            run_cancellable_generation(
                &state,
                &req.session_id,
                generation_agent,
                prompt.to_string(),
                chat_request,
                Some(on_text_delta),
                Some(on_tool_event),
                project_cwd,
                None,
                Some(&req.user_message_id),
            )
            .await
        }
    };

    let _ = app.emit(
        "gen://status",
        serde_json::json!({
            "phase": "response",
            "session_id": &req.session_id,
        }),
    );

    match result {
        Ok(resp) => {
            maybe_extract_session_memory(&state, &app, &req.session_id, &resp.usage);
            let blocks = snapshot_stream_blocks(&stream_blocks);
            let conn = state.conn()?;
            finalize_generate_assistant_message(
                &app,
                &conn,
                &req.session_id,
                &req.user_message_id,
                &params,
                resp,
                blocks,
                &state.role_states,
                &state.file_snapshots,
                &state.token_logger,
                generation_agent,
                &log_model,
                &log_provider,
            )
        }
        Err(AppError::Canceled) => Err(AppError::Canceled),
        Err(e) => {
            let conn = state.conn()?;
            let blocks = snapshot_stream_blocks(&stream_blocks);
            persist_streamed_assistant_snapshot(
                &conn,
                &req.session_id,
                &blocks,
                None,
                None,
                serde_json::json!({ "partial_before_error": true }),
                &state.file_snapshots,
            )?;
            let msg_text = format!("{}", e);
            let err_msg = session::insert_message(
                &conn,
                &req.session_id,
                "error",
                Some(&msg_text),
                Some(params_json.as_str()),
            )?;
            let conn = state.conn()?;
            let user_full = reload_message(&conn, &req.user_message_id)?;
            Ok(GenerateResult {
                user_message: decorate_message(&app, user_full),
                assistant_message: decorate_message(&app, err_msg),
            })
        }
    }
}

/// True when the session still carries the placeholder title, i.e. it has not
/// been renamed (manually or automatically) yet.
fn session_title_is_default(conn: &db::DbConn, id: &str) -> AppResult<bool> {
    let cur: Option<String> = conn
        .query_row(
            "SELECT title FROM sessions WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok();
    Ok(match cur {
        Some(t) => t == "New session" || t.trim().is_empty(),
        None => false,
    })
}

/// Normalise raw LLM output into a usable session title: keep the first line,
/// strip surrounding quotes/punctuation, and cap the length.
fn sanitize_generated_title(raw: &str) -> String {
    let first_line = raw.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let trimmed = first_line.trim().trim_matches(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '"' | '\'' | '`' | '「' | '」' | '『' | '』' | '《' | '》' | '。' | '.' | '：' | ':'
            )
    });
    trimmed.chars().take(40).collect()
}

/// Generate a session title with the configured quick model and persist it.
/// Best-effort: any failure (no response, provider error, concurrent rename)
/// silently leaves the existing title untouched.
async fn generate_title_with_quick_model(
    app: AppHandle,
    state: Arc<AppState>,
    session_id: String,
    prompt: String,
    provider: chat::ProviderConfig,
    model: String,
) {
    let system_prompt = "你是一个会话标题助手。请用不超过 12 个汉字（或 6 个英文单词）概括用户这条消息的主题，作为简短的会话标题。只输出标题本身，不要添加引号、标点、前缀或任何解释。".to_string();
    let request = chat::ChatRequest {
        provider,
        model,
        prompt,
        attachments: Vec::new(),
        system_prompt,
        history: Vec::new(),
        parameters: parameters::factory().build(
            "auto".into(),
            "auto".into(),
            settings::ModelParamSettings::default(),
        ),
        tools: Vec::new(),
        tool_chain: Vec::new(),
        tool_results: Vec::new(),
        pending_assistant_turn: None,
    };

    let factory = crate::ai::providers::ProviderFactory::default();
    let title = match factory.chat(request).await {
        Ok(resp) => sanitize_generated_title(&resp.text.unwrap_or_default()),
        Err(err) => {
            eprintln!("[atelier] quick-model title generation failed: {err}");
            String::new()
        }
    };
    if title.is_empty() {
        return;
    }

    let Ok(conn) = state.conn() else {
        return;
    };
    // The user (or a fallback) may have renamed the session while we waited.
    if !session_title_is_default(&conn, &session_id).unwrap_or(false) {
        return;
    }
    if session::rename(&conn, &session_id, &title).is_ok() {
        let _ = app.emit(
            "session://title",
            serde_json::json!({ "session_id": session_id, "title": title }),
        );
    }
}

fn update_session_title_if_default(conn: &db::DbConn, id: &str, prompt: &str) -> AppResult<()> {
    let cur: Option<String> = conn
        .query_row(
            "SELECT title FROM sessions WHERE id=?1",
            rusqlite::params![id],
            |r| r.get(0),
        )
        .ok();
    if let Some(t) = cur {
        if t == "New session" || t.trim().is_empty() {
            let snippet: String = prompt.chars().take(28).collect();
            let title = if snippet.is_empty() {
                "New session".to_string()
            } else {
                snippet
            };
            conn.execute(
                "UPDATE sessions SET title=?1 WHERE id=?2",
                rusqlite::params![title, id],
            )?;
        }
    }
    Ok(())
}

fn reload_message(conn: &db::DbConn, id: &str) -> AppResult<session::Message> {
    let mut stmt = conn.prepare(
        "SELECT id, session_id, role, text, params_json, created_at FROM messages WHERE id=?1",
    )?;
    let mut rows = stmt.query(rusqlite::params![id])?;
    if let Some(r) = rows.next()? {
        let params_str: Option<String> = r.get(4)?;
        let mut m = session::Message {
            id: r.get(0)?,
            session_id: r.get(1)?,
            role: r.get(2)?,
            text: r.get(3)?,
            params: params_str.and_then(|s| serde_json::from_str(&s).ok()),
            created_at: r.get(5)?,
            images: vec![],
        };
        let mut s = conn.prepare(
            "SELECT id, role, rel_path, thumb_path, mime, width, height, bytes, ord
             FROM message_images WHERE message_id=?1 ORDER BY ord ASC",
        )?;
        let it = s.query_map(rusqlite::params![id], |r| {
            Ok(session::ImageRef {
                id: r.get(0)?,
                role: r.get(1)?,
                rel_path: r.get(2)?,
                thumb_rel_path: r.get(3)?,
                mime: r.get(4)?,
                width: r.get(5)?,
                height: r.get(6)?,
                bytes: r.get(7)?,
                ord: r.get(8)?,
            })
        })?;
        for x in it {
            m.images.push(x?);
        }
        Ok(m)
    } else {
        Err(AppError::NotFound(format!("message {id}")))
    }
}

// ????????? Local edit ?????????

#[derive(Debug, Deserialize)]
struct EditImageArgs {
    image_id: String,
    op: editor::EditOp,
}

#[tauri::command]
fn edit_image(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    args: EditImageArgs,
) -> Result<ImageRefAbs, AppError> {
    let img = {
        let conn = state.conn()?;
        session::get_image(&conn, &args.image_id)?
    };
    let bytes = images::read_image_bytes(&app, &img)?;
    let result = editor::apply(&bytes, &img.mime, &args.op)?;
    let session_id = {
        let conn = state.conn()?;
        session::image_session_id(&conn, &args.image_id)?
    };
    let conn = state.conn()?;
    let new_ref =
        images::write_edited_image(&app, &conn, &session_id, &result.bytes, &result.mime)?;
    Ok(decorate_image(&app, new_ref))
}

// ????????? Export ?????????

#[derive(Debug, Deserialize)]
struct ExportArgs {
    image_id: String,
    dest_path: String,
}

#[tauri::command]
fn export_image(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    args: ExportArgs,
) -> Result<(), AppError> {
    let img = {
        let conn = state.conn()?;
        session::get_image(&conn, &args.image_id)?
    };
    let abs = paths::abs_from_rel(&app, &img.rel_path)?;
    std::fs::copy(&abs, PathBuf::from(&args.dest_path))?;
    Ok(())
}

// ─── Project / Session Transfer (export + import) ────────────────────────────

#[tauri::command]
fn export_projects_archive(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    project_ids: Vec<String>,
    dest_path: String,
) -> Result<(), AppError> {
    crate::data::transfer::export_projects(&app, &state.pool, &project_ids, &dest_path)
}

#[tauri::command]
fn export_session_archive(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    session_id: String,
    dest_path: String,
) -> Result<(), AppError> {
    crate::data::transfer::export_session(&app, &state.pool, &session_id, &dest_path)
}

#[tauri::command]
fn import_archive(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    archive_path: String,
) -> Result<crate::data::transfer::ImportResult, AppError> {
    crate::data::transfer::import_archive(&app, &state.pool, &archive_path)
}

// ─── Decorated DTOs (with abs_path) ──────────────────────────────────────────

#[derive(Debug, Serialize)]
struct ImageRefAbs {
    id: String,
    role: String,
    rel_path: String,
    thumb_rel_path: Option<String>,
    abs_path: String,
    thumb_abs_path: Option<String>,
    mime: String,
    width: Option<i64>,
    height: Option<i64>,
    bytes: Option<i64>,
    ord: i64,
}

#[derive(Debug, Serialize)]
struct MessageAbs {
    id: String,
    session_id: String,
    role: String,
    text: Option<String>,
    params: Option<serde_json::Value>,
    created_at: i64,
    images: Vec<ImageRefAbs>,
}

#[derive(Debug, Serialize)]
struct SessionWithMessagesAbs {
    session: session::Session,
    messages: Vec<MessageAbs>,
}

fn decorate_image(app: &AppHandle, i: session::ImageRef) -> ImageRefAbs {
    let abs = paths::abs_from_rel(app, &i.rel_path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let thumb_abs = i.thumb_rel_path.as_ref().and_then(|r| {
        paths::abs_from_rel(app, r)
            .ok()
            .map(|p| p.to_string_lossy().to_string())
    });
    ImageRefAbs {
        id: i.id,
        role: i.role,
        rel_path: i.rel_path,
        thumb_rel_path: i.thumb_rel_path,
        abs_path: abs,
        thumb_abs_path: thumb_abs,
        mime: i.mime,
        width: i.width,
        height: i.height,
        bytes: i.bytes,
        ord: i.ord,
    }
}

fn decorate_message(app: &AppHandle, m: session::Message) -> MessageAbs {
    MessageAbs {
        id: m.id,
        session_id: m.session_id,
        role: m.role,
        text: m.text,
        params: m.params,
        created_at: m.created_at,
        images: m
            .images
            .into_iter()
            .map(|i| decorate_image(app, i))
            .collect(),
    }
}

fn decorate_session(app: &AppHandle, s: session::SessionWithMessages) -> SessionWithMessagesAbs {
    SessionWithMessagesAbs {
        session: s.session,
        messages: s
            .messages
            .into_iter()
            .map(|m| decorate_message(app, m))
            .collect(),
    }
}

// ????????? Run ?????????

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            let handle = app.handle();
            let db_path = paths::db_path(handle)?;
            let pool = db::open_pool(&db_path)?;

            // Build the shared services first.
            let registry = Arc::new(AgentRegistry::with_builtins());
            let task_store = Arc::new(TaskStore::new());
            let mcp = Arc::new(StaticMcpRegistry::new());
            let provider_engine = Arc::new(ProviderEngine::new());
            let user_context = Arc::new(FsUserContextLoader::new(UserContextConfig::from_env()));

            // Pool starts with the built-in filesystem tools. Wrap in
            // `Arc` immediately so we can register `AgentTool` self-
            // referentially below.
            let tools: Arc<ToolPool> = Arc::new(ToolPool::new());
            let file_snapshots = Arc::new(FileSnapshotStore::new());
            tools.register(FileReadTool::new());
            tools.register(crate::ai::agent::tools::list_files::ListFilesTool::new());
            tools.register(crate::ai::agent::tools::grep::GrepTool::new());
            tools.register(crate::ai::agent::tools::edit::FileWriteTool::new(
                file_snapshots.clone(),
            ));
            tools.register(crate::ai::agent::tools::edit::FileEditTool::new(
                file_snapshots.clone(),
            ));
            tools.register(crate::ai::agent::tools::create_doc::CreateDocTool::new(
                file_snapshots.clone(),
            ));
            tools.register(crate::ai::agent::tools::delete::DeleteTool::new(
                file_snapshots.clone(),
            ));
            tools.register(crate::ai::agent::tools::bash::BashTool::new());
            tools.register_todo_list(crate::ai::agent::tools::todo::TodoListTool::new());
            let role_states = Arc::new(RoleStateStore::new());
            tools.register(RoleStateTool::new(role_states.clone()));
            tools.register(crate::ai::agent::tools::rpg_choice::RpgChoiceTool::new());

            // Build the agent-callable `Agent` tool. The chat factory
            // lets it materialise a sub-agent `ChatRequest` from the
            // current settings on demand and (when
            // `definition.omit_claude_md == false`) attaches CLAUDE.md
            // as a system-reminder.
            let chat_factory: Arc<dyn ChatRequestFactory> =
                Arc::new(SettingsChatFactory::new(pool.clone(), user_context.clone()));
            // Plan-mode aware resolver: in Plan-mode any write tool /
            // mutating Bash invocation is denied at the executor before
            // hitting the tool itself.
            let permission_resolver: Arc<dyn agent::PermissionResolver> = Arc::new(
                crate::ai::agent::core::permission::PlanModeResolver::new(agent::AllowAllResolver),
            );
            let query_engine: Arc<dyn agent::QueryEngine> = Arc::new(
                ProviderQueryEngine::new(provider_engine.clone(), permission_resolver),
            );
            let agent_tool = AgentTool::new(
                registry.clone(),
                tools.clone(),
                task_store.clone(),
                query_engine.clone(),
                mcp.clone(),
            )
            .with_chat_factory(chat_factory);
            tools.register(agent_tool);

            let logs_dir = paths::token_logs_dir()?;
            let token_logger = Arc::new(token_log::TokenUsageLogger::new(
                pool.clone(),
                logs_dir,
            ));

            app.manage(Arc::new(AppState {
                pool,
                generation_abort: Mutex::new(HashMap::new()),
                agent_registry: registry,
                task_store,
                notifications: Arc::new(NotificationQueue::new()),
                engine: provider_engine,
                query_engine,
                user_context,
                mcp,
                tools,
                session_memory: Arc::new(FsSessionMemoryExtractor::new()),
                role_states,
                file_snapshots,
                token_logger,
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            update_settings,
            get_llm_model_catalog,
            fetch_provider_models,
            get_app_info,
            open_path,
            toggle_devtools,
            list_sessions,
            search_sessions,
            create_session,
            rename_session,
            update_session_config,
            set_session_model,
            set_session_agent_type,
            set_session_agent_chain,
            set_project_agent_chain,
            delete_session,
            load_session,
            list_projects,
            create_project,
            rename_project,
            update_project_path,
            delete_project,
            reorder_projects,
            assign_session_to_project,
            update_project_config,
            delete_message,
            update_message_text,
            update_message_images,
            quote_message_as_attachments,
            add_attachment_from_path,
            add_attachment_from_bytes,
            remove_attachment_draft,
            get_image_abs_path,
            cancel_generation,
            list_agent_tasks,
            cancel_agent_task,
            list_agents,
            get_agent_definition,
            list_custom_agents,
            create_custom_agent,
            update_custom_agent,
            delete_custom_agent,
            refresh_user_context,
            set_mcp_servers,
            list_agent_tools,
            get_role_states,
            extract_session_memory,
            get_token_usage_summary,
            list_token_usage_events,
            generate_image,
            regenerate_image,
            save_cancelled_message,
            edit_image,
            export_image,
            export_projects_archive,
            export_session_archive,
            import_archive,
            write_project_file,
            read_project_file,
            project_fs::list_project_dir,
            project_fs::create_project_dir,
            project_fs::create_project_file,
            project_fs::rename_project_path,
            project_fs::copy_project_path,
            project_fs::delete_project_path,
            project_rules::list_project_rules,
            project_rules::set_project_rule_enabled,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
