use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

pub const APP_SUBDIR: &str = "atelier";

/// User-visible root under Documents (e.g. `C:\Users\<user>\Documents\MoYanAgent`).
pub const MOYAN_DOCS_ROOT: &str = "MoYanAgent";
pub const MOYAN_LOGS_DIR: &str = "logs";
pub const MOYAN_PROJECTS_DIR: &str = "Project";

fn user_documents_dir() -> AppResult<PathBuf> {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| AppError::Config("cannot resolve user home directory".into()))?;
    Ok(home.join("Documents"))
}

/// `Documents/MoYanAgent` — created on first use.
pub fn user_moyan_root() -> AppResult<PathBuf> {
    let dir = user_documents_dir()?.join(MOYAN_DOCS_ROOT);
    std::fs::create_dir_all(&dir).map_err(|e| {
        AppError::Other(format!(
            "failed to create MoYanAgent root {}: {e}",
            dir.display()
        ))
    })?;
    Ok(dir)
}

/// `Documents/MoYanAgent/logs/{session_id}.jsonl` — per-session token JSONL logs.
pub fn token_logs_dir() -> AppResult<PathBuf> {
    let dir = user_moyan_root()?.join(MOYAN_LOGS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| {
        AppError::Other(format!(
            "failed to create token logs directory {}: {e}",
            dir.display()
        ))
    })?;
    Ok(dir)
}

/// `Documents/MoYanAgent/Project` — auto-created blank project folders.
pub fn blank_projects_root() -> AppResult<PathBuf> {
    let dir = user_moyan_root()?.join(MOYAN_PROJECTS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| {
        AppError::Other(format!(
            "failed to create blank projects root {}: {e}",
            dir.display()
        ))
    })?;
    Ok(dir)
}

pub fn root_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Config(format!("app_data_dir: {e}")))?;
    let dir = base.join(APP_SUBDIR);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn db_path(app: &AppHandle) -> AppResult<PathBuf> {
    Ok(root_dir(app)?.join("atelier.db"))
}

pub fn sessions_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = root_dir(app)?.join("sessions");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn session_dir(app: &AppHandle, session_id: &str) -> AppResult<PathBuf> {
    let dir = sessions_dir(app)?.join(session_id);
    std::fs::create_dir_all(dir.join("in"))?;
    std::fs::create_dir_all(dir.join("out"))?;
    std::fs::create_dir_all(dir.join("edit"))?;
    std::fs::create_dir_all(dir.join("thumb"))?;
    Ok(dir)
}

pub fn rel_to_root(app: &AppHandle, abs: &Path) -> AppResult<String> {
    let root = root_dir(app)?;
    let rel = abs
        .strip_prefix(&root)
        .map_err(|_| AppError::Invalid("path is outside app data".into()))?;
    Ok(rel.to_string_lossy().replace('\\', "/"))
}

pub fn abs_from_rel(app: &AppHandle, rel: &str) -> AppResult<PathBuf> {
    let root = root_dir(app)?;
    let p = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
    Ok(p)
}
