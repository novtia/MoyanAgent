use std::path::PathBuf;
use std::sync::Arc;

use crate::ai::agent::tools::text_decode::{
    read_text_file, write_text_file_labeled, ProjectTextFile,
};
use crate::data::db;
use crate::error::{AppError, AppResult};

use super::reader_paths::{session_project_cwd, validate_reader_write_path};
use super::state::AppState;

#[tauri::command]
pub fn write_project_file(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    path: String,
    content: String,
    encoding: Option<String>,
    had_bom: Option<bool>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let file_path = PathBuf::from(&path);
    let cwd = session_project_cwd(&conn, &session_id);
    let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;

    write_text_file_labeled(&resolved, &content, encoding.as_deref(), had_bom)
        .map_err(|e| AppError::Other(format!("write_project_file: write {:?}: {e}", resolved)))?;
    Ok(())
}

#[tauri::command]
pub fn read_project_file(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    path: String,
) -> Result<ProjectTextFile, AppError> {
    let conn = state.conn()?;
    let file_path = PathBuf::from(&path);
    let cwd = session_project_cwd(&conn, &session_id);
    let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;
    read_text_file(&resolved)
        .map(ProjectTextFile::from)
        .map_err(|e| AppError::Other(format!("read_project_file: read {:?}: {e}", resolved)))
}

pub(crate) fn apply_pending_diff_revert(
    conn: &db::DbConn,
    session_id: &str,
    revert: &crate::data::pending_diff::PendingDiffRevert,
) -> AppResult<()> {
    let file_path = PathBuf::from(&revert.path);
    let cwd = session_project_cwd(conn, session_id);
    let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;
    write_text_file_labeled(
        &resolved,
        &revert.text,
        revert.encoding.as_deref(),
        Some(revert.had_bom),
    )
    .map_err(|e| {
        AppError::Other(format!(
            "confirm_pending_diff: write {:?}: {e}",
            resolved
        ))
    })?;
    Ok(())
}

#[tauri::command]
pub fn list_pending_diffs(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    path: Option<String>,
) -> Result<Vec<crate::data::pending_diff::PendingDiffRow>, AppError> {
    let conn = state.conn()?;
    match path {
        Some(p) if !p.is_empty() => crate::data::pending_diff::list_for_path(&conn, &session_id, &p),
        _ => crate::data::pending_diff::list_for_session(&conn, &session_id),
    }
}

#[tauri::command]
pub fn confirm_pending_diff(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    id: String,
    accept: bool,
) -> Result<Option<crate::data::pending_diff::PendingDiffRevert>, AppError> {
    let conn = state.conn()?;
    if accept {
        crate::data::pending_diff::accept(&conn, &session_id, &id)?;
        return Ok(None);
    }
    let Some(revert) = crate::data::pending_diff::reject(&conn, &session_id, &id)? else {
        return Ok(None);
    };
    apply_pending_diff_revert(&conn, &session_id, &revert)?;
    Ok(Some(revert))
}

#[tauri::command]
pub fn confirm_all_pending_diffs(
    state: tauri::State<Arc<AppState>>,
    session_id: String,
    path: String,
    accept: bool,
) -> Result<Option<crate::data::pending_diff::PendingDiffRevert>, AppError> {
    let conn = state.conn()?;
    if accept {
        crate::data::pending_diff::accept_all(&conn, &session_id, &path)?;
        return Ok(None);
    }
    let Some(revert) = crate::data::pending_diff::reject_all(&conn, &session_id, &path)? else {
        return Ok(None);
    };
    apply_pending_diff_revert(&conn, &session_id, &revert)?;
    Ok(Some(revert))
}
