//! Persistence for pending Edit review hunks (reader Keep/Undo UI).
//!
//! Unlike [`crate::data::file_snapshot`] (message-level rollback pre-images),
//! these rows are the authoritative source for the post-apply review UI and
//! survive tab close / app restart until the user accepts or rejects them.
//! They also participate in delete-message rollback via `request_message_id`
//! / `message_id` so disk can be restored even when `file_snapshots` are missing.

use std::collections::HashMap;

use rusqlite::params;
use serde::Serialize;
use ulid::Ulid;

use crate::data::db::{now_ms, DbConn};
use crate::error::AppResult;

/// Max combined UTF-8 bytes of `text_before` + `text_after` we will store.
/// Matches the file-snapshot pre-image cap so huge files skip the review UI.
pub const MAX_PENDING_DIFF_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDiffRow {
    pub id: String,
    pub session_id: String,
    pub path: String,
    pub before_snippet: String,
    pub after_snippet: String,
    pub text_before: String,
    pub text_after: String,
    pub encoding: Option<String>,
    pub had_bom: bool,
    pub seq: i64,
    pub created_at: i64,
    pub request_message_id: Option<String>,
    pub message_id: Option<String>,
}

/// Result of rejecting one or all hunks: content to write back to disk.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDiffRevert {
    pub path: String,
    pub text: String,
    pub encoding: Option<String>,
    pub had_bom: bool,
}

fn row_from_query(r: &rusqlite::Row<'_>) -> rusqlite::Result<PendingDiffRow> {
    let had_bom: i64 = r.get(8)?;
    Ok(PendingDiffRow {
        id: r.get(0)?,
        session_id: r.get(1)?,
        path: r.get(2)?,
        before_snippet: r.get(3)?,
        after_snippet: r.get(4)?,
        text_before: r.get(5)?,
        text_after: r.get(6)?,
        encoding: r.get(7)?,
        had_bom: had_bom != 0,
        seq: r.get(9)?,
        created_at: r.get(10)?,
        request_message_id: r.get(11)?,
        message_id: r.get(12)?,
    })
}

const SELECT_COLS: &str = "id, session_id, path, before_snippet, after_snippet, \
     text_before, text_after, encoding, had_bom, seq, created_at, \
     request_message_id, message_id";

/// Insert a new pending hunk. Returns `None` when the payloads exceed the
/// size cap (caller should omit `pending_diff_id` from the tool result).
pub fn insert(
    conn: &DbConn,
    session_id: &str,
    path: &str,
    before_snippet: &str,
    after_snippet: &str,
    text_before: &str,
    text_after: &str,
    encoding: Option<&str>,
    had_bom: bool,
    request_message_id: Option<&str>,
) -> AppResult<Option<String>> {
    let total = text_before.len().saturating_add(text_after.len());
    if total > MAX_PENDING_DIFF_BYTES {
        return Ok(None);
    }

    let next_seq: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM pending_diffs
             WHERE session_id = ?1 AND path = ?2",
            params![session_id, path],
            |r| r.get(0),
        )
        .unwrap_or(1);

    let id = Ulid::new().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO pending_diffs(
            id, session_id, path, before_snippet, after_snippet,
            text_before, text_after, encoding, had_bom, seq, created_at,
            request_message_id, message_id)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL)",
        params![
            id,
            session_id,
            path,
            before_snippet,
            after_snippet,
            text_before,
            text_after,
            encoding,
            had_bom as i64,
            next_seq,
            now,
            request_message_id,
        ],
    )?;
    Ok(Some(id))
}

/// Bind unbound rows from this user turn to the finalized assistant message.
pub fn bind_message(
    conn: &DbConn,
    session_id: &str,
    request_message_id: &str,
    assistant_message_id: &str,
) -> AppResult<usize> {
    let n = conn.execute(
        "UPDATE pending_diffs
         SET message_id = ?1
         WHERE session_id = ?2
           AND request_message_id = ?3
           AND message_id IS NULL",
        params![assistant_message_id, session_id, request_message_id],
    )?;
    Ok(n)
}

/// Bind every unbound pending-diff row in the session to an assistant message
/// (used when the originating user message id is unknown, e.g. cancel save).
pub fn bind_unbound(
    conn: &DbConn,
    session_id: &str,
    assistant_message_id: &str,
) -> AppResult<usize> {
    let n = conn.execute(
        "UPDATE pending_diffs
         SET message_id = ?1
         WHERE session_id = ?2 AND message_id IS NULL",
        params![assistant_message_id, session_id],
    )?;
    Ok(n)
}

/// Roll back every pending hunk tied to `message_id` (as assistant id or as
/// the originating user `request_message_id`). Returns one revert per path
/// using the earliest hunk's `text_before`.
pub fn rollback_for_message(
    conn: &DbConn,
    session_id: &str,
    message_id: &str,
) -> AppResult<Vec<PendingDiffRevert>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM pending_diffs
         WHERE session_id = ?1
           AND (message_id = ?2 OR request_message_id = ?2)
         ORDER BY path ASC, seq ASC"
    ))?;
    let rows = stmt.query_map(params![session_id, message_id], row_from_query)?;
    let mut by_path: HashMap<String, PendingDiffRow> = HashMap::new();
    for row in rows {
        let row = row?;
        by_path.entry(row.path.clone()).or_insert(row);
    }

    let mut reverts = Vec::new();
    for (path, first) in by_path {
        reverts.push(PendingDiffRevert {
            path: path.clone(),
            text: first.text_before,
            encoding: first.encoding,
            had_bom: first.had_bom,
        });
    }

    conn.execute(
        "DELETE FROM pending_diffs
         WHERE session_id = ?1
           AND (message_id = ?2 OR request_message_id = ?2)",
        params![session_id, message_id],
    )?;

    Ok(reverts)
}

pub fn list_for_session(conn: &DbConn, session_id: &str) -> AppResult<Vec<PendingDiffRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM pending_diffs
         WHERE session_id = ?1
         ORDER BY path ASC, seq ASC"
    ))?;
    let rows = stmt.query_map(params![session_id], row_from_query)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn list_for_path(
    conn: &DbConn,
    session_id: &str,
    path: &str,
) -> AppResult<Vec<PendingDiffRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM pending_diffs
         WHERE session_id = ?1 AND path = ?2
         ORDER BY seq ASC"
    ))?;
    let rows = stmt.query_map(params![session_id, path], row_from_query)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn get(conn: &DbConn, session_id: &str, id: &str) -> AppResult<Option<PendingDiffRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM pending_diffs
         WHERE session_id = ?1 AND id = ?2"
    ))?;
    let mut rows = stmt.query_map(params![session_id, id], row_from_query)?;
    match rows.next() {
        Some(Ok(row)) => Ok(Some(row)),
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
}

/// Accept a single hunk: delete that row only.
pub fn accept(conn: &DbConn, session_id: &str, id: &str) -> AppResult<bool> {
    let n = conn.execute(
        "DELETE FROM pending_diffs WHERE session_id = ?1 AND id = ?2",
        params![session_id, id],
    )?;
    Ok(n > 0)
}

/// Reject a hunk: return revert text and delete this row plus every later
/// hunk on the same path (seq >= this row).
pub fn reject(
    conn: &DbConn,
    session_id: &str,
    id: &str,
) -> AppResult<Option<PendingDiffRevert>> {
    let Some(row) = get(conn, session_id, id)? else {
        return Ok(None);
    };
    conn.execute(
        "DELETE FROM pending_diffs
         WHERE session_id = ?1 AND path = ?2 AND seq >= ?3",
        params![session_id, row.path, row.seq],
    )?;
    Ok(Some(PendingDiffRevert {
        path: row.path,
        text: row.text_before,
        encoding: row.encoding,
        had_bom: row.had_bom,
    }))
}

pub fn accept_all(conn: &DbConn, session_id: &str, path: &str) -> AppResult<usize> {
    let n = conn.execute(
        "DELETE FROM pending_diffs WHERE session_id = ?1 AND path = ?2",
        params![session_id, path],
    )?;
    Ok(n)
}

/// Reject all hunks on a path: revert to the first hunk's text_before.
pub fn reject_all(
    conn: &DbConn,
    session_id: &str,
    path: &str,
) -> AppResult<Option<PendingDiffRevert>> {
    let rows = list_for_path(conn, session_id, path)?;
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let revert = PendingDiffRevert {
        path: first.path.clone(),
        text: first.text_before.clone(),
        encoding: first.encoding.clone(),
        had_bom: first.had_bom,
    };
    conn.execute(
        "DELETE FROM pending_diffs WHERE session_id = ?1 AND path = ?2",
        params![session_id, path],
    )?;
    Ok(Some(revert))
}

pub fn clear_path(conn: &DbConn, session_id: &str, path: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM pending_diffs WHERE session_id = ?1 AND path = ?2",
        params![session_id, path],
    )?;
    Ok(())
}

pub fn clear_session(conn: &DbConn, session_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM pending_diffs WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// Clear pending review rows for paths that were rolled back via file snapshots.
pub fn clear_paths(conn: &DbConn, session_id: &str, paths: &[String]) -> AppResult<()> {
    for path in paths {
        clear_path(conn, session_id, path)?;
    }
    Ok(())
}
