use std::sync::Arc;

use serde::Deserialize;

use crate::data::{paths, project, session, settings};
use crate::error::AppError;

use super::state::AppState;

#[derive(Debug, Deserialize)]
pub(crate) struct CreateProjectArgs {
    pub(crate) name: String,
    pub(crate) path: Option<String>,
}

#[tauri::command]
pub fn list_projects(
    state: tauri::State<Arc<AppState>>,
) -> Result<Vec<project::Project>, AppError> {
    let conn = state.conn()?;
    project::list(&conn)
}

#[tauri::command]
pub fn create_project(
    app: tauri::AppHandle,
    state: tauri::State<Arc<AppState>>,
    args: CreateProjectArgs,
) -> Result<project::Project, AppError> {
    paths::init_user_roots(&app)?;
    let name = args.name.trim();
    if name.is_empty() {
        return Err(AppError::Invalid("project name is required".into()));
    }
    let supplied = args
        .path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let path = if paths::pe_platform() {
        None
    } else {
        if let Some(p) = supplied {
            paths::assert_allowed_project_path(p)?;
        }
        supplied
    };
    let conn = state.conn()?;
    project::create(&conn, name, path)
}

#[tauri::command]
pub fn rename_project(
    state: tauri::State<Arc<AppState>>,
    id: String,
    name: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    project::rename(&conn, &id, &name)
}

#[tauri::command]
pub fn update_project_path(
    app: tauri::AppHandle,
    state: tauri::State<Arc<AppState>>,
    id: String,
    path: Option<String>,
) -> Result<(), AppError> {
    paths::init_user_roots(&app)?;
    let conn = state.conn()?;
    project::set_path(&conn, &id, path.as_deref())
}

#[tauri::command]
pub fn delete_project(state: tauri::State<Arc<AppState>>, id: String) -> Result<(), AppError> {
    {
        let conn = state.conn()?;
        let _ = crate::data::role_state::clear_scope(&conn, &id);
        project::delete(&conn, &id)?;
    }
    state.role_states.clear(&id);
    Ok(())
}

#[tauri::command]
pub fn reorder_projects(
    state: tauri::State<Arc<AppState>>,
    ordered_ids: Vec<String>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    project::reorder(&conn, &ordered_ids)
}

#[tauri::command]
pub fn assign_session_to_project(
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
    project::assign_session(&conn, &session_id, project_id.as_deref())?;
    // Standalone → always ask/chat. Joining a project upgrades chat → agent
    // so the session can use workspace tools; other modes are left as-is.
    if project_id.is_none() {
        let _ = session::set_agent_type(&conn, &session_id, session::SESSION_AGENT_CHAT);
    } else if let Ok(sess) = session::get(&conn, &session_id) {
        if sess.agent_type == session::SESSION_AGENT_CHAT {
            let _ = session::set_agent_type(&conn, &session_id, session::SESSION_AGENT_GENERAL);
        }
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateProjectConfigArgs {
    pub(crate) id: String,
    pub(crate) system_prompt: String,
    pub(crate) history_turns: i64,
    pub(crate) llm_params: settings::ModelParamSettings,
    pub(crate) context_window: Option<i64>,
}

#[tauri::command]
pub fn update_project_config(
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetProjectAgentChainArgs {
    pub(crate) id: String,
    pub(crate) chain: Vec<session::ChainNode>,
}

#[tauri::command]
pub fn set_project_agent_chain(
    state: tauri::State<Arc<AppState>>,
    args: SetProjectAgentChainArgs,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    project::set_agent_chain(&conn, &args.id, &args.chain)
}
