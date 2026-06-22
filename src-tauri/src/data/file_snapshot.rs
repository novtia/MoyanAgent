//! Persistence for per-message file-mutation snapshots (rollback support).
//!
//! Unlike [`crate::data::role_state`] (one FULL-board row per message), a
//! single assistant message can touch many files, so this table stores one row
//! per file operation. Each row captures the file's pre-image so deleting /
//! regenerating a message can roll the workspace back to what it was before.

use std::path::PathBuf;

use rusqlite::params;

use crate::ai::agent::core::file_snapshot::PendingFileChange;
use crate::data::db::{now_ms, DbConn};
use crate::error::AppResult;

/// One disk-restore action produced by a rollback.
#[derive(Debug, Clone)]
pub struct FileRestore {
    pub path: PathBuf,
    /// `Some(text)` → rewrite the file with this content.
    pub content: Option<String>,
    /// `true` → remove the file (it was created within the rolled-back range).
    pub delete: bool,
}

/// Persist the pending changes captured during one generation, bound to the
/// assistant `message_id`.
pub fn save_changes(
    conn: &DbConn,
    session_id: &str,
    message_id: &str,
    changes: &[PendingFileChange],
) -> AppResult<()> {
    let now = now_ms();
    for c in changes {
        conn.execute(
            "INSERT INTO file_snapshots(
                session_id, message_id, path, op,
                before_existed, before_content, restorable, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                session_id,
                message_id,
                c.path.to_string_lossy(),
                c.op.as_str(),
                c.before_existed as i64,
                c.before_content,
                c.restorable as i64,
                now,
            ],
        )?;
    }
    Ok(())
}

/// Delete the snapshot rows bound to `message_id` AND every row recorded after
/// them in the same session, returning the disk-restore actions to apply.
///
/// Rows are returned newest-first: applying them in order lands each file on
/// its OLDEST pre-image within the rolled-back range (last write wins).
pub fn rollback_from_message(
    conn: &DbConn,
    session_id: &str,
    message_id: &str,
) -> AppResult<Vec<FileRestore>> {
    // MIN(id) is the first row this message recorded; everything from there on
    // (this message + all later messages) gets rolled back.
    let pivot: Option<i64> = conn.query_row(
        "SELECT MIN(id) FROM file_snapshots WHERE message_id = ?1",
        params![message_id],
        |r| r.get(0),
    )?;
    let Some(pivot) = pivot else {
        return Ok(Vec::new());
    };

    let mut restores = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT path, before_existed, before_content, restorable
             FROM file_snapshots
             WHERE session_id = ?1 AND id >= ?2
             ORDER BY id DESC",
        )?;
        let rows = stmt.query_map(params![session_id, pivot], |r| {
            let path: String = r.get(0)?;
            let before_existed: i64 = r.get(1)?;
            let before_content: Option<String> = r.get(2)?;
            let restorable: i64 = r.get(3)?;
            Ok((path, before_existed != 0, before_content, restorable != 0))
        })?;
        for row in rows {
            let (path, before_existed, before_content, restorable) = row?;
            let path = PathBuf::from(path);
            if before_existed {
                // Restore prior content when we captured it; binary/oversized
                // pre-images (restorable == false) can't be rewritten — skip.
                if restorable {
                    restores.push(FileRestore {
                        path,
                        content: before_content,
                        delete: false,
                    });
                }
            } else {
                // File was created within the range → undo = delete it.
                restores.push(FileRestore {
                    path,
                    content: None,
                    delete: true,
                });
            }
        }
    }

    conn.execute(
        "DELETE FROM file_snapshots WHERE session_id = ?1 AND id >= ?2",
        params![session_id, pivot],
    )?;
    Ok(restores)
}

/// Drop every snapshot for a session (e.g. when the session is deleted).
pub fn clear_session(conn: &DbConn, session_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM file_snapshots WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}
