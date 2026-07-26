use std::sync::Arc;

use crate::data::db;
use crate::error::AppError;

use super::state::AppState;

#[tauri::command]
pub fn get_role_states(
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

/// Full-object replace for one role on the board, then persist a manual snapshot.
#[tauri::command]
pub fn update_role_state(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    role: serde_json::Value,
) -> Result<serde_json::Value, AppError> {
    let id = role
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Invalid("update_role_state: role.id is required".into()))?
        .to_string();
    let conn = state.conn()?;
    let scope = crate::data::role_state::resolve_role_state_scope(&conn, &session_id)?;
    // Ensure the in-memory board is hydrated before replace (same as get).
    let live = state.role_states.snapshot(&scope);
    if live.is_empty() {
        let roles = crate::data::role_state::latest_roles(&conn, &scope)?;
        state.role_states.load(&scope, roles);
    }
    let updated = state.role_states.replace(&scope, &id, role)?;
    let board = state.role_states.snapshot(&scope);
    let message_id = format!("manual-{}", db::now_ms());
    crate::data::role_state::save_snapshot(&conn, &scope, &session_id, &message_id, &board)?;
    Ok(updated)
}

/// Persist a new board order for the session's role-state scope.
#[tauri::command]
pub fn reorder_role_states(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    ordered_ids: Vec<String>,
) -> Result<Vec<serde_json::Value>, AppError> {
    let conn = state.conn()?;
    let scope = crate::data::role_state::resolve_role_state_scope(&conn, &session_id)?;
    let live = state.role_states.snapshot(&scope);
    if live.is_empty() {
        let roles = crate::data::role_state::latest_roles(&conn, &scope)?;
        state.role_states.load(&scope, roles);
    }
    let board = state.role_states.reorder(&scope, &ordered_ids)?;
    let message_id = format!("manual-{}", db::now_ms());
    crate::data::role_state::save_snapshot(&conn, &scope, &session_id, &message_id, &board)?;
    Ok(board)
}

/// Remove one role from the board and persist a snapshot (including empty).
#[tauri::command]
pub fn delete_role_state(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    id: String,
) -> Result<serde_json::Value, AppError> {
    let conn = state.conn()?;
    let scope = crate::data::role_state::resolve_role_state_scope(&conn, &session_id)?;
    let live = state.role_states.snapshot(&scope);
    if live.is_empty() {
        let roles = crate::data::role_state::latest_roles(&conn, &scope)?;
        state.role_states.load(&scope, roles);
    }
    let removed = state.role_states.delete(&scope, &id)?;
    // Always snapshot — even `[]` — so get_role_states won't fall back to an
    // older non-empty DB row after the board was cleared.
    let board = state.role_states.snapshot(&scope);
    let message_id = format!("manual-{}", db::now_ms());
    crate::data::role_state::save_snapshot(&conn, &scope, &session_id, &message_id, &board)?;
    Ok(serde_json::json!({ "removed": removed }))
}
