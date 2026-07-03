//! Persistence for the per-message character state board snapshots.
//!
//! Snapshots are keyed by `scope_id` (project_id for project sessions,
//! session_id for standalone sessions). Each row captures the FULL set of
//! roles (a JSON array) as it stood after one assistant message.

use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use crate::data::db::{now_ms, DbConn};
use crate::error::AppResult;

/// Resolve the role-state scope for a session: `project_id` when the session
/// belongs to a project, otherwise the session id itself.
pub fn resolve_role_state_scope(conn: &DbConn, session_id: &str) -> AppResult<String> {
    let project_id: Option<String> = conn
        .query_row(
            "SELECT project_id FROM sessions WHERE id = ?1",
            params![session_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    Ok(project_id.unwrap_or_else(|| session_id.to_string()))
}

/// Persist (or replace) the snapshot bound to `message_id`.
pub fn save_snapshot(
    conn: &DbConn,
    scope_id: &str,
    session_id: &str,
    message_id: &str,
    roles: &[Value],
) -> AppResult<()> {
    let state_json = serde_json::to_string(roles).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO role_state_snapshots(scope_id, session_id, message_id, state_json, created_at)
         VALUES(?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(message_id) DO UPDATE SET
            scope_id = excluded.scope_id,
            session_id = excluded.session_id,
            state_json = excluded.state_json,
            created_at = excluded.created_at",
        params![scope_id, session_id, message_id, state_json, now_ms()],
    )?;
    Ok(())
}

/// The most recent snapshot for a scope (the current board), or an empty
/// vector when none exists.
pub fn latest_roles(conn: &DbConn, scope_id: &str) -> AppResult<Vec<Value>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT state_json FROM role_state_snapshots
             WHERE scope_id = ?1 ORDER BY id DESC LIMIT 1",
            params![scope_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(parse_roles(raw))
}

/// Delete the snapshot bound to `message_id` AND every snapshot recorded after
/// it in the same scope (rollback semantics). Returns the new latest board.
pub fn rollback_from_message(
    conn: &DbConn,
    scope_id: &str,
    message_id: &str,
) -> AppResult<Vec<Value>> {
    let pivot: Option<i64> = conn
        .query_row(
            "SELECT id FROM role_state_snapshots WHERE message_id = ?1",
            params![message_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(pivot) = pivot {
        conn.execute(
            "DELETE FROM role_state_snapshots WHERE scope_id = ?1 AND id >= ?2",
            params![scope_id, pivot],
        )?;
    }
    latest_roles(conn, scope_id)
}

/// Drop every snapshot for a scope (standalone session or deleted project).
pub fn clear_scope(conn: &DbConn, scope_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM role_state_snapshots WHERE scope_id = ?1",
        params![scope_id],
    )?;
    Ok(())
}

/// When a session joins a project, re-tag snapshots that were scoped to the
/// session id so they become part of the shared project board.
pub fn reassign_session_scope(
    conn: &DbConn,
    session_id: &str,
    project_id: &str,
) -> AppResult<()> {
    conn.execute(
        "UPDATE role_state_snapshots SET scope_id = ?1 WHERE scope_id = ?2",
        params![project_id, session_id],
    )?;
    Ok(())
}

fn parse_roles(raw: Option<String>) -> Vec<Value> {
    raw.and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
        .unwrap_or_default()
}
