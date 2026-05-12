use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::oneshot;

use crate::ai::agent::tools::agent_tool::{AgentTool, ChatRequestFactory};
use crate::ai::agent::exec::engine::ProviderQueryEngine;
use crate::ai::agent::memory::UserContextLoader;
use crate::ai::agent::tools::ToolPool;
use crate::ai::agent::{
    self, AgentRegistry, FileReadTool, FsSessionMemoryExtractor, FsUserContextLoader,
    NotificationQueue, ProviderEngine, StaticMcpRegistry, Task, TaskState, TaskStore,
    UserContextConfig,
};
use crate::ai::{chat, parameters, router};
use crate::data::db::DbPool;
use crate::data::{db, llm_catalog, paths, session, settings};
use crate::error::{AppError, AppResult};
use crate::media::{editor, images};

pub struct AppState {
    pub pool: DbPool,
    generation_cancel: Mutex<HashMap<String, oneshot::Sender<()>>>,

    /// Agent subsystem services. Kept on `AppState` so that any Tauri
    /// command can register tasks, fan out notifications, or pick up a
    /// definition without walking the global registry every call.
    pub agent_registry: Arc<AgentRegistry>,
    pub task_store: Arc<TaskStore>,
    pub notifications: Arc<NotificationQueue>,
    pub engine: Arc<ProviderEngine>,
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
}

impl AppState {
    fn conn(&self) -> AppResult<db::DbConn> {
        Ok(self.pool.get()?)
    }
}

fn generation_cancel_lock(
    state: &AppState,
) -> AppResult<MutexGuard<'_, HashMap<String, oneshot::Sender<()>>>> {
    state
        .generation_cancel
        .lock()
        .map_err(|_| AppError::Other("generation cancellation lock poisoned".into()))
}

fn register_generation_cancel(
    state: &AppState,
    session_id: &str,
) -> AppResult<oneshot::Receiver<()>> {
    let (tx, rx) = oneshot::channel();
    let mut guard = generation_cancel_lock(state)?;
    if guard.contains_key(session_id) {
        return Err(AppError::Invalid(
            "generation already in progress for session".into(),
        ));
    }
    guard.insert(session_id.to_string(), tx);
    Ok(rx)
}

fn clear_generation_cancel(state: &AppState, session_id: &str) {
    if let Ok(mut guard) = state.generation_cancel.lock() {
        guard.remove(session_id);
    }
}

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
        .filter(|m| {
            // Skip empty turns (no text and no usable images) �?they don't add context.
            let has_text = m
                .text
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false);
            let has_img = m
                .images
                .iter()
                .any(|i| matches!(i.role.as_str(), "input" | "output" | "edited"));
            has_text || has_img
        })
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
        out.push(chat::HistoryTurn {
            role: m.role.clone(),
            text: m.text.clone(),
            images: payload,
        });
    }
    Ok(out)
}

/// Route a chat request through the agent layer.
///
/// Every generation now flows through [`agent::run_chat_request`], which
/// registers a [`Task`] against [`AppState::task_store`]. The per-session
/// oneshot channel (`generation_cancel`) is kept for the existing
/// frontend cancel API �?it races against the agent call and, on cancel,
/// marks the task as `Killed` before returning.
async fn run_cancellable_generation(
    state: &AppState,
    session_id: &str,
    agent_type: &str,
    prompt: String,
    mut request: chat::ChatRequest,
    on_text_delta: Option<chat::TextDeltaCallback>,
) -> AppResult<chat::GenerateResponse> {
    let cancel_rx = register_generation_cancel(state, session_id)?;

    // Drain any pending task-notifications addressed to the main loop and
    // prepend them to the chat history as hidden user-meta turns. This
    // mirrors how `query.ts` injects `<task-notification>` at turn
    // boundaries so the model sees background results on the *next* call.
    let drained = state.notifications.drain_for_main();
    if !drained.is_empty() {
        crate::ai::agent::exec::engine::inject_attachments_into_history(&mut request, &drained);
    }

    // Prepend the rendered user-context (CLAUDE.md / rules / MEMORY.md)
    // as the very first hidden user-meta history turn. This mirrors
    // `prependUserContext()` on the TS side: it sits in front of every
    // other turn so the model always sees the active instructions.
    if let Ok(ctx) = state.user_context.load() {
        if !ctx.rendered.trim().is_empty() {
            let mut head = vec![chat::HistoryTurn {
                role: "user".into(),
                text: Some(ctx.rendered.clone()),
                images: Vec::new(),
            }];
            head.append(&mut request.history);
            request.history = head;
        }
    }

    let engine = state.engine.clone();
    let store = state.task_store.clone();
    let prompt_for_agent = prompt;
    let agent_type = agent_type.to_string();

    let result = tokio::select! {
        outcome = async move {
            agent::run_chat_request(
                engine.as_ref(),
                store.as_ref(),
                &agent_type,
                prompt_for_agent,
                request,
                on_text_delta,
            )
            .await
            .map(|o| (o.response, o.task_id))
        } => result_from_outcome(outcome, state),
        _ = cancel_rx => Err(AppError::Canceled),
    };
    clear_generation_cancel(state, session_id);
    result
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

fn result_from_outcome(
    outcome: AppResult<(chat::GenerateResponse, agent::TaskId)>,
    _state: &AppState,
) -> AppResult<chat::GenerateResponse> {
    outcome.map(|(resp, _tid)| resp)
}

/// Post-sampling hook: ask the session-memory extractor whether it
/// wants to refresh `summary.md` for this session. Mirrors the
/// `extractSessionMemory()` post-sampling pass �?non-blocking, best-effort.
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

fn stream_text_callback(
    app: AppHandle,
    session_id: String,
    request_message_id: String,
) -> chat::TextDeltaCallback {
    Arc::new(move |delta| {
        let _ = app.emit(
            "gen://stream",
            serde_json::json!({
                "session_id": &session_id,
                "request_message_id": &request_message_id,
                "text_delta": delta.text,
                "thinking_delta": delta.thinking,
            }),
        );
    })
}

// ───────── Settings ─────────

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

// ───────── App info ─────────

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

// ───────── Sessions ─────────

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
    session::create(&conn, args.title, args.model)
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
    session::set_model_and_context(
        &conn,
        &args.id,
        Some(args.model.as_str()),
        args.context_window,
    )
}

#[tauri::command]
fn delete_session(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> Result<(), AppError> {
    {
        let conn = state.conn()?;
        session::delete(&conn, &id)?;
    }
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
    let s = session::load_with_messages(&conn, &id)?;
    Ok(decorate_session(&app, s))
}

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
    Ok(())
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

// ───────── Attachments ─────────

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

// ───────── Image asset URL helpers ─────────

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

// ───────── Generate ─────────

#[tauri::command]
fn cancel_generation(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
) -> Result<(), AppError> {
    let mut guard = generation_cancel_lock(&state)?;
    if let Some(tx) = guard.remove(&session_id) {
        let _ = tx.send(());
    }
    Ok(())
}

// ───────── Agent task commands ─────────

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

#[derive(Debug, Deserialize)]
struct GenerateReq {
    session_id: String,
    prompt: String,
    attachment_ids: Vec<String>,
    aspect_ratio: String,
    image_size: String,
}

#[derive(Debug, Serialize)]
struct GenerateResult {
    user_message: MessageAbs,
    assistant_message: MessageAbs,
}

/// Dedupe multimodal duplicates, persist assistant row + output images, return API DTO.
fn finalize_generate_assistant_message(
    app: &AppHandle,
    conn: &db::DbConn,
    session_id: &str,
    user_message_id: &str,
    params: &parameters::GenerationParameters,
    mut resp: chat::GenerateResponse,
) -> AppResult<GenerateResult> {
    resp.images = chat::dedupe_image_results(resp.images);
    let assistant_params_json = params
        .to_assistant_message_params(&resp.usage, resp.thinking_content.as_deref())
        .to_string();
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
    let user_full = reload_message(conn, user_message_id)?;
    let assistant_full = reload_message(conn, &assistant.id)?;
    if let Some(t) = resp.usage.total_tokens.filter(|t| *t > 0) {
        session::bump_context_window_used(conn, session_id, t)?;
    }
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
    let (chat_request, params, attachment_image_ids) = {
        let conn = state.conn()?;
        let s = settings::read(&conn)?;
        let session_config = session::get(&conn, &req.session_id)?;
        let session_prompt = session_config.system_prompt.clone();
        let history_turns = session_config.history_turns;
        let model_params = session_config.llm_params.clone();
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
            session_prompt,
            hist,
            params.clone(),
        )?;
        (chat_request, params, ids)
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
        update_session_title_if_default(&conn, &req.session_id, &req.prompt)?;
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
    let on_text_delta =
        stream_text_callback(app.clone(), req.session_id.clone(), user_msg.id.clone());
    let result = run_cancellable_generation(
        &state,
        &req.session_id,
        agent::config::builtin::AGENT_GENERAL_PURPOSE,
        req.prompt.clone(),
        chat_request,
        Some(on_text_delta),
    )
    .await;

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
            let conn = state.conn()?;
            finalize_generate_assistant_message(
                &app,
                &conn,
                &req.session_id,
                &user_msg.id,
                &params,
                resp,
            )
        }
        Err(AppError::Canceled) => Err(AppError::Canceled),
        Err(e) => {
            let conn = state.conn()?;
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

#[derive(Debug, Deserialize)]
struct RegenerateReq {
    session_id: String,
    user_message_id: String,
    aspect_ratio: String,
    image_size: String,
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

    let (chat_request, params) = {
        let conn = state.conn()?;
        let s = settings::read(&conn)?;
        let session_config = session::get(&conn, &req.session_id)?;
        let session_prompt = session_config.system_prompt.clone();
        let history_turns = session_config.history_turns;
        let model_params = session_config.llm_params.clone();
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
            session_prompt,
            hist,
            params.clone(),
        )?;
        (chat_request, params)
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

    let on_text_delta = stream_text_callback(
        app.clone(),
        req.session_id.clone(),
        req.user_message_id.clone(),
    );
    let result = run_cancellable_generation(
        &state,
        &req.session_id,
        agent::config::builtin::AGENT_GENERAL_PURPOSE,
        prompt.to_string(),
        chat_request,
        Some(on_text_delta),
    )
    .await;

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
            let conn = state.conn()?;
            finalize_generate_assistant_message(
                &app,
                &conn,
                &req.session_id,
                &req.user_message_id,
                &params,
                resp,
            )
        }
        Err(AppError::Canceled) => Err(AppError::Canceled),
        Err(e) => {
            let conn = state.conn()?;
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

// ───────── Local edit ─────────

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

// ───────── Export ─────────

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

// ───────── Decorated DTOs (with abs_path) ─────────

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

// ───────── Run ─────────

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
            tools.register(FileReadTool::new());
            tools.register(crate::ai::agent::tools::edit::FileWriteTool::new());
            tools.register(crate::ai::agent::tools::edit::FileEditTool::new());
            tools.register(crate::ai::agent::tools::bash::BashTool::new());

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
                query_engine,
                mcp.clone(),
            )
            .with_chat_factory(chat_factory);
            tools.register(agent_tool);

            app.manage(Arc::new(AppState {
                pool,
                generation_cancel: Mutex::new(HashMap::new()),
                agent_registry: registry,
                task_store,
                notifications: Arc::new(NotificationQueue::new()),
                engine: provider_engine,
                user_context,
                mcp,
                tools,
                session_memory: Arc::new(FsSessionMemoryExtractor::new()),
            }));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_settings,
            update_settings,
            get_llm_model_catalog,
            get_app_info,
            open_path,
            list_sessions,
            search_sessions,
            create_session,
            rename_session,
            update_session_config,
            set_session_model,
            delete_session,
            load_session,
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
            refresh_user_context,
            set_mcp_servers,
            list_agent_tools,
            extract_session_memory,
            generate_image,
            regenerate_image,
            edit_image,
            export_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
