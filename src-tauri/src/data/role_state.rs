//! Persistence for the per-message character state board snapshots.
//!
//! Each row in `role_state_snapshots` captures the FULL set of roles (a JSON
//! array) as it stood after one assistant message. This lets us:
//!
//! - re-hydrate the in-memory [`crate::ai::agent::RoleStateStore`] when a
//!   session is opened (load the latest snapshot), and
//! - roll the board back when a message is deleted / regenerated (drop that
//!   message's snapshot and any later ones, then reload the new latest).

use rusqlite::{params, OptionalExtension};
use serde_json::Value;

use crate::data::db::{now_ms, DbConn};
use crate::error::AppResult;

/// Persist (or replace) the snapshot bound to `message_id`.
pub fn save_snapshot(
    conn: &DbConn,
    session_id: &str,
    message_id: &str,
    roles: &[Value],
) -> AppResult<()> {
    let state_json = serde_json::to_string(roles).unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT INTO role_state_snapshots(session_id, message_id, state_json, created_at)
         VALUES(?1, ?2, ?3, ?4)
         ON CONFLICT(message_id) DO UPDATE SET
            state_json = excluded.state_json,
            created_at = excluded.created_at",
        params![session_id, message_id, state_json, now_ms()],
    )?;
    Ok(())
}

/// The most recent snapshot for a session (the current board), or an empty
/// vector when none exists.
pub fn latest_roles(conn: &DbConn, session_id: &str) -> AppResult<Vec<Value>> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT state_json FROM role_state_snapshots
             WHERE session_id = ?1 ORDER BY id DESC LIMIT 1",
            params![session_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(parse_roles(raw))
}

/// Delete the snapshot bound to `message_id` AND every snapshot recorded after
/// it in the same session (rollback semantics). Returns the new latest board.
pub fn rollback_from_message(
    conn: &DbConn,
    session_id: &str,
    message_id: &str,
) -> AppResult<Vec<Value>> {
    // Find the row id for this message (if any). Everything with id >= it goes.
    let pivot: Option<i64> = conn
        .query_row(
            "SELECT id FROM role_state_snapshots WHERE message_id = ?1",
            params![message_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(pivot) = pivot {
        conn.execute(
            "DELETE FROM role_state_snapshots WHERE session_id = ?1 AND id >= ?2",
            params![session_id, pivot],
        )?;
    }
    latest_roles(conn, session_id)
}

/// Drop every snapshot for a session (e.g. when the session is deleted).
pub fn clear_session(conn: &DbConn, session_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM role_state_snapshots WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

fn parse_roles(raw: Option<String>) -> Vec<Value> {
    raw.and_then(|s| serde_json::from_str::<Vec<Value>>(&s).ok())
        .unwrap_or_default()
}
