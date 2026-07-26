//! User-initiated project filesystem operations from the reader file explorer.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::data::db;
use crate::error::{AppError, AppResult};

use super::project_rules;
use super::reader_paths::{session_project_cwd, validate_reader_write_path};
use super::state::AppState;

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
    // Inside the `.moyan` rules folder, hide the internal manifest so it reads
    // as a plain folder of rule files.
    let in_rules_dir = dir
        .file_name()
        .map(|n| n == project_rules::RULES_DIR)
        .unwrap_or(false);
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| {
        AppError::Other(format!("list_project_dir: read_dir {:?}: {e}", dir))
    })? {
        let entry = entry.map_err(|e| AppError::Other(format!("list_project_dir: entry: {e}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Hide dotfiles, except the project rules folder itself.
        if name.starts_with('.') && name != project_rules::RULES_DIR {
            continue;
        }
        if in_rules_dir && name == project_rules::RULES_MANIFEST {
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

fn copy_path_recursive(from: &Path, to: &Path) -> AppResult<()> {
    if from.is_dir() {
        fs::create_dir_all(to).map_err(|e| {
            AppError::Other(format!("copy_project_path: mkdir {:?}: {e}", to))
        })?;
        for entry in fs::read_dir(from).map_err(|e| {
            AppError::Other(format!("copy_project_path: read_dir {:?}: {e}", from))
        })? {
            let entry =
                entry.map_err(|e| AppError::Other(format!("copy_project_path: entry: {e}")))?;
            let child_to = to.join(entry.file_name());
            copy_path_recursive(&entry.path(), &child_to)?;
        }
    } else {
        if let Some(parent) = to.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).map_err(|e| {
                    AppError::Other(format!("copy_project_path: mkdir {:?}: {e}", parent))
                })?;
            }
        }
        fs::copy(from, to).map_err(|e| {
            AppError::Other(format!(
                "copy_project_path: copy {:?} -> {:?}: {e}",
                from, to
            ))
        })?;
    }
    Ok(())
}

/// True when `child` is the same as `parent` or nested under it.
fn path_is_within(parent: &Path, child: &Path) -> bool {
    let Ok(parent) = fs::canonicalize(parent) else {
        return false;
    };
    let Ok(child) = fs::canonicalize(child) else {
        // Destination may not exist yet — compare lexical parents when possible.
        let parent_s = parent.to_string_lossy().replace('\\', "/").to_lowercase();
        let child_s = child.to_string_lossy().replace('\\', "/").to_lowercase();
        return child_s == parent_s || child_s.starts_with(&format!("{parent_s}/"));
    };
    child == parent || child.starts_with(&parent)
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
pub fn copy_project_path(
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
            "copy_project_path: source does not exist: {}",
            from_path.display()
        )));
    }
    if to_path.exists() {
        return Err(AppError::Invalid(format!(
            "copy_project_path: destination already exists: {}",
            to_path.display()
        )));
    }
    copy_path_recursive(&from_path, &to_path)
}

/// Copy an OS path (file or folder) into the project at `dest_path`.
///
/// `src_path` is not required to live under the project root. `dest_path` must
/// pass [`validate_reader_write_path`] and must not already exist.
#[tauri::command]
pub fn import_external_path_to_project(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
    src_path: String,
    dest_path: String,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let src = PathBuf::from(src_path.trim());
    if src_path.trim().is_empty() || !src.exists() {
        return Err(AppError::Invalid(format!(
            "import_external_path_to_project: source does not exist: {}",
            src.display()
        )));
    }

    let dest = resolve_validated_path(&conn, &session_id, &dest_path)?;
    if dest.exists() {
        return Err(AppError::Invalid(format!(
            "import_external_path_to_project: destination already exists: {}",
            dest.display()
        )));
    }

    if src.is_dir() {
        // Reject copying a folder into itself or a descendant path.
        if let Some(parent) = dest.parent() {
            if path_is_within(&src, parent) || path_is_within(&src, &dest) {
                return Err(AppError::Invalid(
                    "import_external_path_to_project: cannot import a folder into itself".into(),
                ));
            }
        }
    }

    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::Other(format!(
                    "import_external_path_to_project: mkdir {:?}: {e}",
                    parent
                ))
            })?;
        }
    }

    copy_path_recursive(&src, &dest).map_err(|e| match e {
        AppError::Other(msg) => AppError::Other(
            msg.replace("copy_project_path:", "import_external_path_to_project:"),
        ),
        other => other,
    })
}

/// Write raw bytes to a new project file (HTML5 drop fallback when `File.path` is unavailable).
#[tauri::command]
pub fn write_project_file_bytes(
    state: State<'_, std::sync::Arc<AppState>>,
    session_id: String,
    path: String,
    bytes: Vec<u8>,
) -> Result<(), AppError> {
    let conn = state.conn()?;
    let resolved = resolve_validated_path(&conn, &session_id, &path)?;
    if resolved.exists() {
        return Err(AppError::Invalid(format!(
            "write_project_file_bytes: destination already exists: {}",
            resolved.display()
        )));
    }
    if let Some(parent) = resolved.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::Other(format!(
                    "write_project_file_bytes: mkdir {:?}: {e}",
                    parent
                ))
            })?;
        }
    }
    fs::write(&resolved, &bytes).map_err(|e| {
        AppError::Other(format!(
            "write_project_file_bytes: write {:?}: {e}",
            resolved
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
