//! Persistence for per-message file-mutation snapshots (rollback support).
//!
//! This table is the single source of truth for undoing workspace writes.
//! Unlike [`crate::data::role_state`] (one FULL-board row per message), a
//! single assistant message can touch many files, so there is one row per file
//! operation, each holding that file's pre-image.
//!
//! Rows are inserted when the tool writes, not when the message finalizes:
//!
//! - `request_message_id` — the user turn that triggered the write. Set
//!   immediately, so a cancelled or crashed generation stays attributable.
//! - `message_id` — the assistant message that owns the write. Filled in by
//!   [`bind_message`] once that message exists; `NULL` until then.
//!
//! Rollback therefore matches on *either* id, and unbound rows are reclaimed
//! when their user turn is deleted or regenerated.

use std::collections::HashSet;

use rusqlite::params;

use crate::ai::agent::core::file_snapshot::FileChangeRecord;
use crate::data::db::{now_ms, DbConn};
use crate::error::AppResult;

/// One disk-restore action produced by a rollback.
#[derive(Debug, Clone)]
pub struct FileRestore {
    /// `file_snapshots.id`, so a failed restore can be reported back.
    pub row_id: i64,
    pub path: String,
    /// `Some(text)` → rewrite the file with this content.
    pub content: Option<String>,
    pub encoding: Option<String>,
    pub had_bom: bool,
    /// `true` → remove the file (it was created within the rolled-back range).
    pub delete: bool,
}

/// The rows a rollback covers, plus the disk actions to apply.
///
/// Handed back to [`finish_rollback`] after the caller has attempted the
/// actions, so rows only disappear once the disk actually matches them.
#[derive(Debug, Clone)]
pub struct RollbackPlan {
    /// Lowest `id` in the rolled-back range.
    pub pivot: i64,
    /// Newest-first: applying in order lands each file on its OLDEST pre-image
    /// within the range (last write wins).
    pub restores: Vec<FileRestore>,
    /// Rows whose file existed but whose pre-image was never captured (too
    /// large to store). There is no disk action to attempt, yet the file *is*
    /// still modified, so these are reported as rollback failures rather than
    /// dropped — otherwise the row vanishes and the user is told the undo
    /// succeeded while the file keeps the new content.
    pub unrestorable: Vec<(i64, String)>,
}

/// Persist one pre-image captured just before a tool touches disk.
pub fn record_change(
    conn: &DbConn,
    session_id: &str,
    request_message_id: Option<&str>,
    change: &FileChangeRecord,
) -> AppResult<()> {
    let encoding_label = change.before_encoding.map(|e| e.label().to_string());
    conn.execute(
        "INSERT INTO file_snapshots(
            session_id, message_id, request_message_id, path, op,
            before_existed, before_content, restorable,
            before_encoding, before_had_bom, created_at)
         VALUES(?1, NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            session_id,
            request_message_id,
            change.path,
            change.op.as_str(),
            change.before_existed as i64,
            change.before_content,
            change.restorable as i64,
            encoding_label,
            change.before_had_bom as i64,
            now_ms(),
        ],
    )?;
    Ok(())
}

/// Bind this turn's unbound rows to the assistant message that produced them.
///
/// Rows carrying a *different* `request_message_id` are deliberately left
/// alone: they belong to an earlier turn whose generation never finalized, and
/// mis-attributing them here would roll back the wrong range later.
///
/// Rows with no `request_message_id` at all (a tool that ran without a
/// correlation id) are claimed only when they were recorded after this turn's
/// user message. Without that bound, every stale orphan in the session — some
/// from turns the user may since have kept — would be swept into this message
/// and undone the moment it is deleted.
pub fn bind_message(
    conn: &DbConn,
    session_id: &str,
    request_message_id: &str,
    assistant_message_id: &str,
) -> AppResult<usize> {
    let n = conn.execute(
        "UPDATE file_snapshots
         SET message_id = ?1
         WHERE session_id = ?2
           AND message_id IS NULL
           AND (
             request_message_id = ?3
             OR (request_message_id IS NULL
                 AND created_at >= COALESCE(
                   (SELECT created_at FROM messages WHERE id = ?3), created_at))
           )",
        params![assistant_message_id, session_id, request_message_id],
    )?;
    Ok(n)
}

/// Bind every unbound row in the session (used when the originating user
/// message id is unknown, e.g. saving a cancelled turn).
pub fn bind_unbound(
    conn: &DbConn,
    session_id: &str,
    assistant_message_id: &str,
) -> AppResult<usize> {
    let n = conn.execute(
        "UPDATE file_snapshots
         SET message_id = ?1
         WHERE session_id = ?2 AND message_id IS NULL",
        params![assistant_message_id, session_id],
    )?;
    Ok(n)
}

/// Plan the rollback of `message_id` AND every mutation recorded after it in
/// the same session. `message_id` may be the owning assistant message or the
/// user turn that requested it, which is what lets a regenerate reclaim writes
/// left behind by a generation that never finalized.
///
/// Nothing is deleted here — call [`finish_rollback`] once the disk actions
/// have been attempted.
pub fn plan_rollback_from_message(
    conn: &DbConn,
    session_id: &str,
    message_id: &str,
) -> AppResult<Option<RollbackPlan>> {
    // MIN(id) is the first row this turn recorded; everything from there on
    // (this message + all later messages) gets rolled back.
    let pivot: Option<i64> = conn.query_row(
        "SELECT MIN(id) FROM file_snapshots
         WHERE session_id = ?1 AND (message_id = ?2 OR request_message_id = ?2)",
        params![session_id, message_id],
        |r| r.get(0),
    )?;
    let Some(pivot) = pivot else {
        return Ok(None);
    };

    let mut restores = Vec::new();
    let mut unrestorable = Vec::new();
    let mut stmt = conn.prepare(
        "SELECT id, path, before_existed, before_content, restorable,
                before_encoding, before_had_bom
         FROM file_snapshots
         WHERE session_id = ?1 AND id >= ?2
         ORDER BY id DESC",
    )?;
    let rows = stmt.query_map(params![session_id, pivot], |r| {
        let row_id: i64 = r.get(0)?;
        let path: String = r.get(1)?;
        let before_existed: i64 = r.get(2)?;
        let before_content: Option<String> = r.get(3)?;
        let restorable: i64 = r.get(4)?;
        let before_encoding: Option<String> = r.get(5)?;
        let before_had_bom: i64 = r.get(6)?;
        Ok((
            row_id,
            path,
            before_existed != 0,
            before_content,
            restorable != 0,
            before_encoding,
            before_had_bom != 0,
        ))
    })?;
    for row in rows {
        let (row_id, path, before_existed, before_content, restorable, before_encoding, had_bom) =
            row?;
        if !before_existed {
            restores.push(FileRestore {
                row_id,
                path,
                content: None,
                encoding: None,
                had_bom: false,
                delete: true,
            });
        } else if restorable {
            restores.push(FileRestore {
                row_id,
                path,
                content: before_content,
                encoding: before_encoding,
                had_bom,
                delete: false,
            });
        } else {
            unrestorable.push((
                row_id,
                format!(
                    "{path}: the previous content was too large to snapshot, so this change \
                     could not be undone — restore the file manually"
                ),
            ));
        }
    }
    drop(stmt);

    Ok(Some(RollbackPlan {
        pivot,
        restores,
        unrestorable,
    }))
}

/// Retire a planned rollback: drop the rows whose disk action succeeded and
/// keep the ones that failed, stamped with the reason.
///
/// Keeping failures means a rollback that could not run (path outside the
/// project, file locked, drive gone) stays visible and is retried the next
/// time an earlier message is rolled back, instead of silently vanishing.
pub fn finish_rollback(
    conn: &DbConn,
    session_id: &str,
    plan: &RollbackPlan,
    failures: &[(i64, String)],
) -> AppResult<()> {
    conn.execute(
        "UPDATE file_snapshots SET rollback_error = NULL
         WHERE session_id = ?1 AND id >= ?2",
        params![session_id, plan.pivot],
    )?;
    let mut seen = HashSet::new();
    for (row_id, message) in failures {
        if !seen.insert(*row_id) {
            continue;
        }
        conn.execute(
            "UPDATE file_snapshots SET rollback_error = ?1 WHERE id = ?2",
            params![message, row_id],
        )?;
    }
    conn.execute(
        "DELETE FROM file_snapshots
         WHERE session_id = ?1 AND id >= ?2 AND rollback_error IS NULL",
        params![session_id, plan.pivot],
    )?;
    Ok(())
}

/// Drop every snapshot for a session (e.g. when the session is deleted).
pub fn clear_session(conn: &DbConn, session_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM file_snapshots WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::agent::core::file_snapshot::FileOp;
    use crate::data::db::test_support::TempDb;

    const SID: &str = "session-1";

    fn created(path: &str) -> FileChangeRecord {
        FileChangeRecord {
            path: path.to_string(),
            op: FileOp::Create,
            before_existed: false,
            before_content: None,
            before_encoding: None,
            before_had_bom: false,
            restorable: true,
        }
    }

    fn updated(path: &str, before: &str) -> FileChangeRecord {
        FileChangeRecord {
            path: path.to_string(),
            op: FileOp::Update,
            before_existed: true,
            before_content: Some(before.to_string()),
            before_encoding: None,
            before_had_bom: false,
            restorable: true,
        }
    }

    #[test]
    fn a_created_file_rolls_back_as_a_delete() {
        let db = TempDb::new("fs-create");
        let conn = db.conn();
        record_change(&conn, SID, Some("u1"), &created("proj/new.md")).unwrap();
        bind_message(&conn, SID, "u1", "a1").unwrap();

        let plan = plan_rollback_from_message(&conn, SID, "a1")
            .unwrap()
            .expect("assistant message owns a snapshot");
        assert_eq!(plan.restores.len(), 1);
        assert!(plan.restores[0].delete);

        finish_rollback(&conn, SID, &plan, &[]).unwrap();
        assert!(plan_rollback_from_message(&conn, SID, "a1")
            .unwrap()
            .is_none());
    }

    #[test]
    fn an_unfinalized_turn_is_reclaimed_through_its_user_message() {
        let db = TempDb::new("fs-orphan");
        let conn = db.conn();
        // Generation cancelled before any assistant message existed.
        record_change(&conn, SID, Some("u1"), &created("proj/new.md")).unwrap();

        let plan = plan_rollback_from_message(&conn, SID, "u1")
            .unwrap()
            .expect("orphan is still attributable to its user turn");
        assert_eq!(plan.restores.len(), 1);
        assert!(plan.restores[0].delete);
    }

    #[test]
    fn binding_leaves_an_earlier_turns_orphan_alone() {
        let db = TempDb::new("fs-bind");
        let conn = db.conn();
        record_change(&conn, SID, Some("u1"), &created("proj/one.md")).unwrap();
        record_change(&conn, SID, Some("u2"), &created("proj/two.md")).unwrap();

        assert_eq!(bind_message(&conn, SID, "u2", "a2").unwrap(), 1);

        // a2 only owns its own write...
        let bound = plan_rollback_from_message(&conn, SID, "a2")
            .unwrap()
            .expect("a2 plan");
        assert_eq!(bound.restores.len(), 1);
        assert_eq!(bound.restores[0].path, "proj/two.md");

        // ...while u1's orphan still anchors a rollback of everything after it.
        let from_orphan = plan_rollback_from_message(&conn, SID, "u1")
            .unwrap()
            .expect("u1 plan");
        assert_eq!(from_orphan.restores.len(), 2);
    }

    /// A file that was modified but never snapshot must not disappear from the
    /// record: the undo did not actually happen for it.
    #[test]
    fn an_uncapturable_pre_image_is_reported_instead_of_silently_dropped() {
        let db = TempDb::new("fs-unrestorable");
        let conn = db.conn();
        let mut huge = updated("proj/huge.md", "");
        huge.before_content = None;
        huge.restorable = false;
        record_change(&conn, SID, Some("u1"), &huge).unwrap();
        bind_message(&conn, SID, "u1", "a1").unwrap();

        let plan = plan_rollback_from_message(&conn, SID, "a1")
            .unwrap()
            .expect("a1 plan");
        assert!(plan.restores.is_empty(), "nothing can be written back");
        assert_eq!(plan.unrestorable.len(), 1, "but the caller must be told");

        finish_rollback(&conn, SID, &plan, &plan.unrestorable).unwrap();
        let kept = plan_rollback_from_message(&conn, SID, "a1")
            .unwrap()
            .expect("the row survives so the failure stays visible");
        assert_eq!(kept.unrestorable.len(), 1);
    }

    /// Orphans predating this turn belong to messages the user may have kept;
    /// sweeping them into the new assistant message would undo them later.
    #[test]
    fn binding_ignores_correlation_less_rows_from_before_this_turn() {
        let db = TempDb::new("fs-bind-null");
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions(id, title, created_at, updated_at) VALUES(?1, 'x', 0, 0)",
            params![SID],
        )
        .unwrap();
        // The current turn's user message, dated after the stale row.
        conn.execute(
            "INSERT INTO messages(id, session_id, role, created_at) VALUES('u2', ?1, 'user', 500)",
            params![SID],
        )
        .unwrap();

        record_change(&conn, SID, None, &created("proj/stale.md")).unwrap();
        conn.execute(
            "UPDATE file_snapshots SET created_at = 100 WHERE path = 'proj/stale.md'",
            [],
        )
        .unwrap();
        record_change(&conn, SID, None, &created("proj/fresh.md")).unwrap();
        conn.execute(
            "UPDATE file_snapshots SET created_at = 900 WHERE path = 'proj/fresh.md'",
            [],
        )
        .unwrap();

        assert_eq!(
            bind_message(&conn, SID, "u2", "a2").unwrap(),
            1,
            "only the row recorded during this turn is claimed"
        );
        let bound = plan_rollback_from_message(&conn, SID, "a2")
            .unwrap()
            .expect("a2 plan");
        assert_eq!(bound.restores.len(), 1);
        assert_eq!(bound.restores[0].path, "proj/fresh.md");
    }

    #[test]
    fn a_failed_restore_is_kept_with_its_reason_and_can_be_retried() {
        let db = TempDb::new("fs-failure");
        let conn = db.conn();
        record_change(&conn, SID, Some("u1"), &created("proj/locked.md")).unwrap();
        record_change(&conn, SID, Some("u1"), &updated("proj/ok.md", "before")).unwrap();
        bind_message(&conn, SID, "u1", "a1").unwrap();

        let plan = plan_rollback_from_message(&conn, SID, "a1")
            .unwrap()
            .expect("a1 plan");
        let locked = plan
            .restores
            .iter()
            .find(|r| r.path == "proj/locked.md")
            .expect("locked row");
        finish_rollback(
            &conn,
            SID,
            &plan,
            &[(locked.row_id, "file in use".to_string())],
        )
        .unwrap();

        let reason: Option<String> = conn
            .query_row(
                "SELECT rollback_error FROM file_snapshots WHERE id = ?1",
                params![locked.row_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reason.as_deref(), Some("file in use"));

        let retry = plan_rollback_from_message(&conn, SID, "a1")
            .unwrap()
            .expect("failed row stays rollback-able");
        assert_eq!(retry.restores.len(), 1);
        assert_eq!(retry.restores[0].path, "proj/locked.md");
    }
}
