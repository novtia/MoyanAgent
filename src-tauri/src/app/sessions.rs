use std::sync::Arc;

use serde::Deserialize;
use tauri::AppHandle;

use crate::data::{llm_catalog, paths, project, session, settings};
use crate::error::AppError;

use super::dto::{decorate_session, SessionWithMessagesAbs};
use super::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct CreateSessionArgs {
    pub(crate) title: Option<String>,
    pub(crate) model: Option<String>,
}

#[tauri::command]
pub fn list_sessions(
    state: tauri::State<Arc<AppState>>,
) -> Result<Vec<session::SessionSummary>, AppError> {
    let conn = state.conn()?;
    session::list(&conn)
}

#[tauri::command]
pub fn search_sessions(
    state: tauri::State<Arc<AppState>>,
    query: String,
    limit: i64,
) -> Result<Vec<session::SessionSearchResult>, AppError> {
    let conn = state.conn()?;
    session::search(&conn, &query, limit)
}

#[tauri::command]
pub fn create_session(
    state: tauri::State<Arc<AppState>>,
    args: CreateSessionArgs,
) -> Result<session::Session, AppError> {
    let conn = state.conn()?;
    let mut sess = session::create(&conn, args.title, args.model)?;

    if let Ok(s) = settings::read(&conn) {
        // Inherit the global default model + provider (the "new-session
        // template") and its catalog context-window when no explicit model was
        // provided, so each session owns its own model identity from the start.
        if sess.model.is_none() {
            let model = s.model.trim().to_string();
            if !model.is_empty() {
                let provider = settings::active_provider(&s);
                let provider_id = provider.map(|p| p.id.clone()).unwrap_or_default();
                let sdk = provider.map(|p| p.sdk.as_str()).unwrap_or("");
                let cw = llm_catalog::lookup_context_window(&conn, &provider_id, sdk, &model)
                    .ok()
                    .flatten();
                let _ = session::set_provider_model_and_context(
                    &conn,
                    &sess.id,
                    Some(provider_id.as_str()),
                    Some(model.as_str()),
                    cw,
                );
                sess.provider_id = Some(provider_id);
                sess.model = Some(model);
                sess.context_window = cw;
            }
        }

        // Seed the composer thinking default into the session's own llm_params
        // so thinking is self-owned per session from creation onward.
        if s.default_thinking_enabled || !s.default_thinking_effort.trim().is_empty() {
            let mut llm = sess.llm_params.clone();
            llm.thinking_enabled = Some(s.default_thinking_enabled);
            let effort = s.default_thinking_effort.trim();
            llm.thinking_effort = if effort.is_empty() {
                None
            } else {
                Some(effort.to_string())
            };
            if session::update_config(&conn, &sess.id, &sess.system_prompt, sess.history_turns, &llm)
                .is_ok()
            {
                sess.llm_params = llm;
            }
        }
    }

    Ok(sess)
}

#[tauri::command]
pub fn rename_session(
    state: tauri::State<Arc<AppState>>,
    id: String,
    title: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::rename(&conn, &id, &title)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateSessionConfigArgs {
    pub(crate) id: String,
    pub(crate) system_prompt: String,
    pub(crate) history_turns: i64,
    pub(crate) llm_params: settings::ModelParamSettings,
}

#[tauri::command]
pub fn update_session_config(
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
pub(crate) struct SetSessionModelArgs {
    pub(crate) id: String,
    pub(crate) model: String,
    /// Provider the model belongs to. Empty/absent falls back to the global
    /// active provider (legacy callers).
    #[serde(default)]
    pub(crate) provider_id: Option<String>,
    pub(crate) context_window: Option<i64>,
}

#[tauri::command]
pub fn set_session_model(
    state: tauri::State<Arc<AppState>>,
    args: SetSessionModelArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let s = settings::read(&conn)?;
    // Resolve the provider this model belongs to: explicit arg wins, else the
    // global active provider (keeps legacy callers working).
    let provider_id = args
        .provider_id
        .as_ref()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| s.active_provider_id.clone());
    let mut cw = args.context_window;
    if cw.is_none() {
        let sdk = s
            .model_services
            .iter()
            .find(|p| p.id == provider_id)
            .map(|p| p.sdk.as_str())
            .unwrap_or("");
        cw = llm_catalog::lookup_context_window(&conn, &provider_id, sdk, &args.model)?;
    }
    session::set_provider_model_and_context(
        &conn,
        &args.id,
        Some(provider_id.as_str()),
        Some(args.model.as_str()),
        cw,
    )
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetSessionAgentTypeArgs {
    pub(crate) id: String,
    pub(crate) agent_type: String,
}

#[tauri::command]
pub fn set_session_agent_type(
    state: tauri::State<Arc<AppState>>,
    args: SetSessionAgentTypeArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::set_agent_type(&conn, &args.id, &args.agent_type)
}

#[tauri::command]
pub fn delete_session(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
) -> Result<(), AppError> {
    let child_ids = {
        let conn = state.conn()?;
        session::list_temp_child_ids(&conn, &id).unwrap_or_default()
    };
    // Best-effort remote Responses cache cleanup before rows disappear.
    {
        let conn = state.conn()?;
        let s = settings::read(&conn).ok();
        let mut ids = child_ids.clone();
        ids.push(id.clone());
        for sid in ids {
            if let Ok(sess) = session::get(&conn, &sid) {
                if let Some(resp_id) = sess
                    .last_response_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                {
                    let provider = s.as_ref().and_then(|settings| {
                        sess.provider_id
                            .as_ref()
                            .and_then(|pid| {
                                settings.model_services.iter().find(|p| p.id == *pid)
                            })
                            .or_else(|| {
                                settings
                                    .model_services
                                    .iter()
                                    .find(|p| p.id == settings.active_provider_id)
                            })
                    });
                    if let Some(p) = provider {
                        let endpoint = p.endpoint.clone();
                        let api_key = p.api_key.clone();
                        let resp_id = resp_id.to_string();
                        tauri::async_runtime::spawn(async move {
                            crate::ai::providers::openai::delete_stored_response(
                                &endpoint, &api_key, &resp_id,
                            )
                            .await;
                        });
                    }
                }
            }
        }
    }
    {
        let conn = state.conn()?;
        // Explicitly remove temp children first (FK CASCADE is best-effort
        // for columns added via ALTER TABLE).
        for child_id in &child_ids {
            let _ = crate::data::file_snapshot::clear_session(&conn, child_id);
            let _ = crate::data::pending_diff::clear_session(&conn, child_id);
            let _ = session::delete(&conn, child_id);
        }
        let scope = crate::data::role_state::resolve_role_state_scope(&conn, &id)?;
        session::delete(&conn, &id)?;
        // Standalone sessions own their scope; project sessions share scope.
        if scope == id {
            let _ = crate::data::role_state::clear_scope(&conn, &scope);
            state.role_states.clear(&scope);
        }
        let _ = crate::data::file_snapshot::clear_session(&conn, &id);
        let _ = crate::data::pending_diff::clear_session(&conn, &id);
    }
    for child_id in &child_ids {
        state.file_snapshots.clear(child_id);
        state.session_logger.delete_session_log(child_id);
        let dir = paths::sessions_dir(&app)?.join(child_id);
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    state.file_snapshots.clear(&id);
    state.session_logger.delete_session_log(&id);
    let dir = paths::sessions_dir(&app)?.join(&id);
    if dir.exists() {
        let _ = std::fs::remove_dir_all(&dir);
    }
    Ok(())
}

#[tauri::command]
pub fn load_session(
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

fn hydrate_session_chain(
    conn: &crate::data::db::DbConn,
    s: &mut session::SessionWithMessages,
) -> Result<(), AppError> {
    if let Some(ref pid) = s.session.project_id {
        if let Ok(proj) = project::get(conn, pid) {
            s.session.agent_chain = proj.agent_chain;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn list_message_outline(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
) -> Result<Vec<session::MessageOutlineItem>, AppError> {
    let conn = state.conn()?;
    session::list_message_outline(&conn, &session_id)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ListMessagesWindowArgs {
    pub(crate) session_id: String,
    pub(crate) around_message_id: Option<String>,
    pub(crate) before_created_at: Option<i64>,
    pub(crate) after_created_at: Option<i64>,
    pub(crate) limit: Option<i64>,
}

#[tauri::command]
pub fn list_messages_window(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    args: ListMessagesWindowArgs,
) -> Result<Vec<super::dto::MessageAbs>, AppError> {
    let conn = state.conn()?;
    let limit = args.limit.unwrap_or(60);
    let messages = session::load_messages_ordered(
        &conn,
        &args.session_id,
        args.around_message_id.as_deref(),
        args.before_created_at,
        args.after_created_at,
        limit,
    )?;
    Ok(messages
        .into_iter()
        .map(|m| super::dto::decorate_message(&app, m))
        .collect())
}

/// Session metadata + a message window (default: last N). Prefer this over
/// full `load_session` for long chats; pair with `list_message_outline`.
#[tauri::command]
pub fn load_session_window(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    id: String,
    around_message_id: Option<String>,
    limit: Option<i64>,
) -> Result<SessionWithMessagesAbs, AppError> {
    let conn = state.conn()?;
    let scope = crate::data::role_state::resolve_role_state_scope(&conn, &id)?;
    if let Ok(roles) = crate::data::role_state::latest_roles(&conn, &scope) {
        state.role_states.load(&scope, roles);
    }
    let mut s = session::load_with_message_window(
        &conn,
        &id,
        around_message_id.as_deref(),
        limit.unwrap_or(60),
    )?;
    hydrate_session_chain(&conn, &mut s)?;
    Ok(decorate_session(&app, s))
}

#[tauri::command]
pub fn list_session_media(
    state: tauri::State<Arc<AppState>>,
    app: AppHandle,
    session_id: String,
) -> Result<Vec<super::dto::ImageRefAbs>, AppError> {
    let conn = state.conn()?;
    let images = session::list_session_media(&conn, &session_id)?;
    Ok(images
        .into_iter()
        .map(|i| super::dto::decorate_image(&app, i))
        .collect())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetSessionAgentChainArgs {
    pub(crate) id: String,
    pub(crate) chain: Vec<session::ChainNode>,
}

#[tauri::command]
pub fn set_session_agent_chain(
    state: tauri::State<Arc<AppState>>,
    args: SetSessionAgentChainArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::set_agent_chain(&conn, &args.id, &args.chain)
}
