use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, Emitter};

use crate::ai::agent::tools::text_decode::{write_text_file_labeled, TextEncoding};
use crate::data::{db, paths, session};
use crate::error::{AppError, AppResult};

use super::dto::{decorate_message, MessageAbs};
use super::project_io::apply_pending_diff_revert;
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
        state
            .session_logger
            .rollback_from_message(&conn, sid, &id);
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
        // Roll the workspace back: restore / delete every file this message (and
        // any later ones) created, updated or removed.
        let mut restored_paths: Vec<String> = Vec::new();
        if let Ok(restores) = crate::data::file_snapshot::rollback_from_message(&conn, &sid, &id) {
            for r in &restores {
                if let Err(e) = apply_file_restore(&conn, &sid, r) {
                    eprintln!("delete_message: apply_file_restore failed: {e}");
                }
                let raw = r.path.to_string_lossy();
                restored_paths.push(
                    raw.strip_prefix(r"\\?\")
                        .unwrap_or(raw.as_ref())
                        .to_string(),
                );
            }
        }
        // Always also roll back via pending_diffs (covers missing snapshots and
        // deletes of the originating user message via request_message_id).
        match crate::data::pending_diff::rollback_for_message(&conn, &sid, &id) {
            Ok(reverts) => {
                for revert in &reverts {
                    if let Err(e) = apply_pending_diff_revert(&conn, &sid, revert) {
                        eprintln!("delete_message: pending_diff revert failed: {e}");
                    }
                    restored_paths.push(revert.path.clone());
                }
            }
            Err(e) => {
                eprintln!("delete_message: pending_diff rollback_for_message failed: {e}");
            }
        }
        restored_paths.sort();
        restored_paths.dedup();
        let _ = crate::data::pending_diff::clear_paths(&conn, &sid, &restored_paths);
    }
    Ok(())
}

/// Apply a single file-snapshot rollback action to disk: delete a file that
/// was created within the rolled-back range, or rewrite a file with its
/// captured pre-image.
pub(crate) fn apply_file_restore(
    conn: &db::DbConn,
    session_id: &str,
    restore: &crate::data::file_snapshot::FileRestore,
) -> AppResult<()> {
    let raw = restore.path.to_string_lossy();
    let stripped = raw.strip_prefix(r"\\?\").unwrap_or(raw.as_ref());
    let file_path = PathBuf::from(stripped);
    let cwd = session_project_cwd(conn, session_id);
    let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;

    if restore.delete {
        let _ = std::fs::remove_file(&resolved);
        return Ok(());
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
        .map_err(|e| {
            AppError::Other(format!("apply_file_restore: write {:?}: {e}", resolved))
        })?;
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
