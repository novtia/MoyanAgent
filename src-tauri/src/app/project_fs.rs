//! User-initiated project filesystem operations from the reader file explorer.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::data::db;
use crate::error::{AppError, AppResult};

use super::{session_project_cwd, validate_reader_write_path, AppState};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectDirEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

fn resolve_list_dir(
    conn: &db::DbConn,
    session_id: &str,
    path: Option<&str>,
) -> AppResult<PathBuf> {
    if let Some(raw) = path.filter(|p| !p.trim().is_empty()) {
        let file_path = PathBuf::from(raw);
        let cwd = session_project_cwd(conn, session_id);
        let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;
        if !resolved.is_dir() {
            return Err(AppError::Invalid(format!(
                "list_project_dir: not a directory: {}",
                resolved.display()
            )));
        }
        return Ok(resolved);
    }

    let cwd = session_project_cwd(conn, session_id).ok_or_else(|| {
        AppError::Invalid("list_project_dir: session has no project folder".into())
    })?;
    validate_reader_write_path(&cwd, Some(cwd.as_path()))?;
    Ok(cwd)
}

fn list_dir_entries(dir: &Path) -> AppResult<Vec<ProjectDirEntry>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| {
        AppError::Other(format!("list_project_dir: read_dir {:?}: {e}", dir))
    })? {
        let entry = entry.map_err(|e| AppError::Other(format!("list_project_dir: entry: {e}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| AppError::Other(format!("list_project_dir: file_type: {e}")))?;
        entries.push(ProjectDirEntry {
            name,
            path: path.to_string_lossy().into_owned(),
            is_dir: file_type.is_dir(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

fn resolve_validated_path(
    conn: &db::DbConn,
    session_id: &str,
    path: &str,
) -> AppResult<PathBuf> {
    let file_path = PathBuf::from(path);
    let cwd = session_project_cwd(conn, session_id);
    validate_reader_write_path(&file_path, cwd.as_deref())
}

fn remove_path_recursive(path: &Path) -> AppResult<()> {
    if path.is_dir() {
        fs::remove_dir_all(path).map_err(|e| {
            AppError::Other(format!("delete_project_path: remove_dir_all {:?}: {e}", path))
        })?;
    } else {
        fs::remove_file(path).map_err(|e| {
            AppError::Other(format!("delete_project_path: remove_file {:?}: {e}", path))
        })?;
    }
    Ok(())
}

#[tauri::command]
pub fn list_project_dir(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
    path: Option<String>,
) -> Result<Vec<ProjectDirEntry>, AppError> {
    let conn = state.conn()?;
    let dir = resolve_list_dir(&conn, &session_id, path.as_deref())?;
    list_dir_entries(&dir)
}

#[tauri::command]
pub fn create_project_dir(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
    path: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let file_path = PathBuf::from(&path);
    let cwd = session_project_cwd(&conn, &session_id);
    validate_reader_write_path(&file_path, cwd.as_deref())?;
    fs::create_dir_all(&file_path).map_err(|e| {
        AppError::Other(format!("create_project_dir: mkdir {:?}: {e}", file_path))
    })?;
    Ok(())
}

#[tauri::command]
pub fn create_project_file(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
    path: String,
    content: Option<String>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let file_path = PathBuf::from(&path);
    let cwd = session_project_cwd(&conn, &session_id);
    let resolved = validate_reader_write_path(&file_path, cwd.as_deref())?;
    if resolved.exists() {
        return Err(AppError::Invalid(format!(
            "create_project_file: already exists: {}",
            resolved.display()
        )));
    }
    if let Some(parent) = resolved.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::Other(format!("create_project_file: mkdir {:?}: {e}", parent))
            })?;
        }
    }
    let bytes = content.unwrap_or_default();
    fs::write(&resolved, bytes.as_bytes()).map_err(|e| {
        AppError::Other(format!("create_project_file: write {:?}: {e}", resolved))
    })?;
    Ok(())
}

#[tauri::command]
pub fn rename_project_path(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
    from: String,
    to: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let from_path = resolve_validated_path(&conn, &session_id, &from)?;
    let to_path = resolve_validated_path(&conn, &session_id, &to)?;
    if !from_path.exists() {
        return Err(AppError::Invalid(format!(
            "rename_project_path: source does not exist: {}",
            from_path.display()
        )));
    }
    if to_path.exists() {
        return Err(AppError::Invalid(format!(
            "rename_project_path: destination already exists: {}",
            to_path.display()
        )));
    }
    if let Some(parent) = to_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::Other(format!("rename_project_path: mkdir {:?}: {e}", parent))
            })?;
        }
    }
    fs::rename(&from_path, &to_path).map_err(|e| {
        AppError::Other(format!(
            "rename_project_path: rename {:?} -> {:?}: {e}",
            from_path, to_path
        ))
    })?;
    Ok(())
}

#[tauri::command]
pub fn delete_project_path(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
    path: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let resolved = resolve_validated_path(&conn, &session_id, &path)?;
    if !resolved.exists() {
        return Err(AppError::Invalid(format!(
            "delete_project_path: path does not exist: {}",
            resolved.display()
        )));
    }
    remove_path_recursive(&resolved)
}
