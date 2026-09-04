use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::ai::agent::tools::text_decode::{write_text_file_labeled, TextEncoding};
use crate::data::{db, paths, session};
use crate::error::{AppError, AppResult};

use super::dto::{decorate_message, MessageAbs};
use super::reader_paths::{session_project_cwd, validate_reader_write_path};
use super::state::AppState;

#[tauri::command]
pub fn update_message_text(
    state: tauri::State<Arc<AppState>>,
    id: String,
    text: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    session::update_message_text(&conn, &id, &text)
}

#[tauri::command]
pub fn update_message_images(
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
pub fn delete_message(
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
        state.session_logger.rollback_from_message(&conn, sid, &id);
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
        rollback_workspace_from_message(&conn, &sid, &id);
    }
    Ok(())
}

/// Undo every file mutation recorded for `message_id` and everything after it
/// in the session, then retire the review rows that described them.
///
/// `file_snapshots` is the only writer here: it is the one place that knows a
/// file was *created* (and must therefore be deleted rather than rewritten),
/// and it now holds a row from the instant the tool touched disk, so an
/// interrupted generation is covered too. `message_id` may be the assistant
/// message that owns the writes or the user turn that requested them.
pub(crate) fn rollback_workspace_from_message(
    conn: &db::DbConn,
    session_id: &str,
    message_id: &str,
) {
    let plan = match crate::data::file_snapshot::plan_rollback_from_message(
        conn, session_id, message_id,
    ) {
        Ok(Some(plan)) => Some(plan),
        Ok(None) => None,
        Err(e) => {
            eprintln!("rollback_workspace: planning failed for {message_id}: {e}");
            None
        }
    };

    let mut restored_paths: Vec<String> = Vec::new();
    if let Some(plan) = plan {
        let mut failures: Vec<(i64, String)> = plan.unrestorable.clone();
        for (_, reason) in &plan.unrestorable {
            eprintln!("rollback_workspace: {reason}");
        }
        for restore in &plan.restores {
            match apply_file_restore(conn, session_id, restore) {
                Ok(()) => restored_paths.push(restore.path.clone()),
                Err(e) => {
                    eprintln!("rollback_workspace: {} failed: {e}", restore.path);
                    failures.push((restore.row_id, e.to_string()));
                }
            }
        }
        if let Err(e) =
            crate::data::file_snapshot::finish_rollback(conn, session_id, &plan, &failures)
        {
            eprintln!("rollback_workspace: finish failed for {message_id}: {e}");
        }
    }

    if let Err(e) = crate::data::pending_diff::clear_for_message(conn, session_id, message_id) {
        eprintln!("rollback_workspace: pending_diff clear failed for {message_id}: {e}");
    }
    restored_paths.sort();
    restored_paths.dedup();
    let _ = crate::data::pending_diff::clear_paths(conn, session_id, &restored_paths);
}

/// Apply a single file-snapshot rollback action to disk: delete a file that
/// was created within the rolled-back range, or rewrite a file with its
/// captured pre-image.
pub(crate) fn apply_file_restore(
    conn: &db::DbConn,
    session_id: &str,
    restore: &crate::data::file_snapshot::FileRestore,
) -> AppResult<()> {
    let file_path = PathBuf::from(&restore.path);
    let cwd = session_project_cwd(conn, session_id);
    let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;

    if restore.delete {
        return match std::fs::remove_file(&resolved) {
            Ok(()) => Ok(()),
            // Already gone is the state we wanted.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AppError::Other(format!(
                "apply_file_restore: remove {:?}: {e}",
                resolved
            ))),
        };
    }
    if let Some(content) = &restore.content {
        let encoding = restore
            .encoding
            .as_deref()
            .map(TextEncoding::parse_label)
            .unwrap_or(TextEncoding::Utf8);
        write_text_file_labeled(
            &resolved,
            content,
            Some(encoding.label()),
            Some(restore.had_bom),
        )
        .map_err(|e| AppError::Other(format!("apply_file_restore: write {:?}: {e}", resolved)))?;
    }
    Ok(())
}

/// Tell the UI to discard its in-memory role board for a scope and re-fetch
/// the persisted truth (used after a rollback / message deletion).
pub(crate) fn emit_role_state_reset(app: &AppHandle, scope_id: &str, session_id: &str) {
    let _ = app.emit(
        "role-state://reset",
        serde_json::json!({
            "scope_id": scope_id,
            "session_id": session_id,
        }),
    );
}

pub(crate) fn reload_message(conn: &db::DbConn, id: &str) -> AppResult<session::Message> {
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
            "SELECT id, role, rel_path, thumb_path, mime, media_role, source_url, width, height, bytes, ord
             FROM message_images WHERE message_id=?1 ORDER BY ord ASC",
        )?;
        let it = s.query_map(rusqlite::params![id], |r| {
            Ok(session::ImageRef {
                id: r.get(0)?,
                role: r.get(1)?,
                rel_path: r.get(2)?,
                thumb_rel_path: r.get(3)?,
                mime: r.get(4)?,
                media_role: r.get(5)?,
                source_url: r.get(6)?,
                width: r.get(7)?,
                height: r.get(8)?,
                bytes: r.get(9)?,
                ord: r.get(10)?,
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

#[cfg(test)]
mod rollback_tests {
    use super::*;
    use crate::ai::agent::core::file_snapshot::{capture_before, FileOp};
    use crate::data::db::test_support::TempDb;
    use crate::data::{file_snapshot, pending_diff};

    fn workspace(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("atelier-rollback-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create workspace");
        std::fs::canonicalize(&dir).expect("canonicalize workspace")
    }

    fn project_session(conn: &db::DbConn, root: &std::path::Path) -> String {
        let project =
            crate::data::project::create(conn, "rollback-test", Some(&root.to_string_lossy()))
                .expect("create project");
        let sess =
            session::create(conn, Some("rollback-test".into()), None).expect("create session");
        conn.execute(
            "UPDATE sessions SET project_id = ?1 WHERE id = ?2",
            rusqlite::params![project.id, sess.id],
        )
        .expect("attach project");
        sess.id
    }

    /// The resend regression: turn 1 creates a file, turn 2 edits it and leaves
    /// a review row behind. Deleting the turns in resend order must leave the
    /// file gone — the review row must not write it back.
    #[test]
    fn rollback_deletes_a_created_file_and_no_review_row_resurrects_it() {
        let root = workspace("create");
        let db = TempDb::new("rollback-create");
        let conn = db.conn();
        let sid = project_session(&conn, &root);
        let file = root.join("chapter.md");

        let create = capture_before(&file, FileOp::Create).unwrap();
        file_snapshot::record_change(&conn, &sid, Some("u1"), &create).unwrap();
        std::fs::write(&file, "v1").unwrap();
        file_snapshot::bind_message(&conn, &sid, "u1", "a1").unwrap();

        let edit = capture_before(&file, FileOp::Update).unwrap();
        file_snapshot::record_change(&conn, &sid, Some("u2"), &edit).unwrap();
        std::fs::write(&file, "v2").unwrap();
        pending_diff::insert(
            &conn,
            &sid,
            &edit.path,
            "v1",
            "v2",
            "v1",
            "v2",
            Some("utf-8"),
            false,
            Some("u2"),
        )
        .unwrap();
        file_snapshot::bind_message(&conn, &sid, "u2", "a2").unwrap();

        // Resend of u1 deletes everything after it, oldest first.
        for message_id in ["a1", "u2", "a2"] {
            rollback_workspace_from_message(&conn, &sid, message_id);
            assert!(
                !file.exists(),
                "file reappeared while rolling back {message_id}"
            );
        }

        assert!(pending_diff::list_for_session(&conn, &sid)
            .unwrap()
            .is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A generation that never produced an assistant message still owns its
    /// writes through the user turn, so regenerating that turn reclaims them.
    #[test]
    fn rollback_reclaims_writes_from_a_turn_that_never_finalized() {
        let root = workspace("orphan");
        let db = TempDb::new("rollback-orphan");
        let conn = db.conn();
        let sid = project_session(&conn, &root);
        let file = root.join("draft.md");

        let create = capture_before(&file, FileOp::Create).unwrap();
        file_snapshot::record_change(&conn, &sid, Some("u1"), &create).unwrap();
        std::fs::write(&file, "half-written").unwrap();

        rollback_workspace_from_message(&conn, &sid, "u1");

        assert!(!file.exists(), "orphaned write was not rolled back");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// An edited file goes back to its pre-image rather than being deleted.
    #[test]
    fn rollback_restores_an_updated_file_to_its_pre_image() {
        let root = workspace("update");
        let db = TempDb::new("rollback-update");
        let conn = db.conn();
        let sid = project_session(&conn, &root);
        let file = root.join("notes.md");
        std::fs::write(&file, "original").unwrap();

        let edit = capture_before(&file, FileOp::Update).unwrap();
        file_snapshot::record_change(&conn, &sid, Some("u1"), &edit).unwrap();
        std::fs::write(&file, "rewritten").unwrap();
        file_snapshot::bind_message(&conn, &sid, "u1", "a1").unwrap();

        rollback_workspace_from_message(&conn, &sid, "a1");

        assert_eq!(std::fs::read_to_string(&file).unwrap(), "original");
        let _ = std::fs::remove_dir_all(&root);
    }
}
