use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};

pub const APP_SUBDIR: &str = "atelier";

/// User-visible root under Documents (e.g. `C:\Users\<user>\Documents\MoYanAgent`).
pub const MOYAN_DOCS_ROOT: &str = "MoYanAgent";
pub const MOYAN_LOGS_DIR: &str = "logs";
pub const MOYAN_PROJECTS_DIR: &str = "Project";

/// User home, or the Android app sandbox when `HOME`/`USERPROFILE` are absent.
pub fn user_home_dir() -> AppResult<PathBuf> {
    #[cfg(target_os = "android")]
    {
        android_sandbox_dir()
    }
    #[cfg(not(target_os = "android"))]
    {
        std::env::var_os("USERPROFILE")
            .or_else(|| std::env::var_os("HOME"))
            .map(PathBuf::from)
            .ok_or_else(|| AppError::Config("cannot resolve user home directory".into()))
    }
}

#[cfg(target_os = "android")]
fn android_sandbox_dir() -> AppResult<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    std::env::current_dir().map_err(|e| {
        AppError::Config(format!("cannot resolve android sandbox directory: {e}"))
    })
}

fn user_documents_dir() -> AppResult<PathBuf> {
    #[cfg(target_os = "android")]
    {
        android_sandbox_dir()
    }
    #[cfg(not(target_os = "android"))]
    {
        Ok(user_home_dir()?.join("Documents"))
    }
}

/// `Documents/MoYanAgent`, preferring Tauri `app_data_dir` on Android.
pub fn user_moyan_root_for_app(app: &AppHandle) -> AppResult<PathBuf> {
    #[cfg(target_os = "android")]
    {
        let dir = app
            .path()
            .app_data_dir()
            .map_err(|e| AppError::Config(format!("app_data_dir: {e}")))?
            .join(MOYAN_DOCS_ROOT);
        std::fs::create_dir_all(&dir).map_err(|e| {
            AppError::Other(format!(
                "failed to create MoYanAgent root {}: {e}",
                dir.display()
            ))
        })?;
        return Ok(dir);
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        user_moyan_root()
    }
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
pub fn token_logs_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = user_moyan_root_for_app(app)?.join(MOYAN_LOGS_DIR);
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

/// Default auto-backup root: `{data_dir}/backups`.
pub fn backups_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = root_dir(app)?.join("backups");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// User skill packages: `{data_dir}/skills/<id>/SKILL.md`.
pub fn skills_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = root_dir(app)?.join("skills");
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

/// Strip Windows extended-length / verbatim prefixes (`\\?\C:\...`, `\\?\UNC\...`)
/// so stored and displayed paths stay user-readable.
pub fn strip_verbatim_prefix(path: &str) -> String {
    let p = path.trim();
    if let Some(rest) = p.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{rest}");
    }
    if let Some(rest) = p.strip_prefix(r"\\?\") {
        return rest.to_string();
    }
    p.to_string()
}

/// Path string for tool JSON / UI: strips Windows verbatim prefixes after canonicalize.
pub fn display_path(path: &Path) -> String {
    strip_verbatim_prefix(&path.to_string_lossy())
}

/// Canonicalize an existing path, or its nearest existing ancestor when the
/// final file / folders have not been created yet.
///
/// On Windows `canonicalize` returns a verbatim `\\?\` path. Comparing that
/// against an untouched non-existent target makes a valid child look like it
/// sits outside the root, so the missing tail is rebuilt on top of the
/// canonical ancestor to keep both sides in one representation.
pub fn canonicalize_with_missing_tail(path: &Path) -> AppResult<PathBuf> {
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(AppError::Invalid(format!(
            "path traversal is not allowed: {}",
            path.display()
        )));
    }

    let mut ancestor = path;
    let mut missing = Vec::new();

    while !ancestor.exists() {
        let segment = ancestor
            .file_name()
            .ok_or_else(|| AppError::Invalid(format!("cannot resolve path {}", path.display())))?;
        if segment == "." || segment == ".." {
            return Err(AppError::Invalid(format!(
                "path traversal is not allowed: {}",
                path.display()
            )));
        }
        missing.push(segment.to_os_string());
        ancestor = ancestor
            .parent()
            .ok_or_else(|| AppError::Invalid(format!("cannot resolve path {}", path.display())))?;
    }

    let mut resolved = std::fs::canonicalize(ancestor)
        .map_err(|e| AppError::Other(format!("canonicalize {}: {e}", ancestor.display())))?;
    for segment in missing.iter().rev() {
        resolved.push(segment);
    }
    Ok(resolved)
}

/// True when `candidate` is `root` itself or sits underneath it.
///
/// Both sides are reduced to one spelling first, because neither a raw string
/// comparison nor `Path::starts_with` is trustworthy here: on Windows
/// `canonicalize` yields a verbatim `\\?\C:\…` path, so comparing it against an
/// un-canonicalized root rejects valid children, and the comparison is
/// case-sensitive while NTFS is not — `c:\project` would look foreign to
/// `C:\Project`.
///
/// A path containing `..` never resolves (see
/// [`canonicalize_with_missing_tail`]) and therefore never passes as contained
/// unless it exists on disk, where canonicalization removes the traversal.
pub fn is_within(root: &Path, candidate: &Path) -> bool {
    fn normalize(path: &Path) -> String {
        let text = canonicalize_with_missing_tail(path)
            .map(|p| display_path(&p))
            .unwrap_or_else(|_| display_path(path));
        if cfg!(windows) {
            text.to_lowercase()
        } else {
            text
        }
    }

    let root = normalize(root);
    let root = root.trim_end_matches(['\\', '/']);
    let candidate = normalize(candidate);
    if candidate == root {
        return true;
    }
    // Compared component-wise so `C:\proj-old` is not read as a child of
    // `C:\proj`.
    Path::new(&candidate).starts_with(Path::new(root))
}

/// The single stored representation of a workspace file.
///
/// `file_snapshots` and `pending_diffs` both key off this string, so rollback
/// and review-row cleanup can match on equality instead of hoping two
/// independently derived path spellings happen to agree. Falls back to the
/// raw path when the location cannot be canonicalized at all (e.g. the drive
/// went away), which keeps a best-effort record rather than dropping one.
pub fn normalized_path_key(path: &Path) -> String {
    canonicalize_with_missing_tail(path)
        .map(|p| display_path(&p))
        .unwrap_or_else(|_| display_path(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_verbatim_drive_and_unc() {
        assert_eq!(strip_verbatim_prefix(r"\\?\C:\Users\x"), r"C:\Users\x");
        assert_eq!(
            strip_verbatim_prefix(r"\\?\UNC\server\share\a"),
            r"\\server\share\a"
        );
        assert_eq!(strip_verbatim_prefix(r"C:\Users\x"), r"C:\Users\x");
    }

    #[test]
    fn normalized_key_agrees_for_existing_and_not_yet_created_files() {
        let root = std::env::temp_dir().join(format!("moyan_key_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create temp dir");
        let file = root.join("章节.md");

        let before_create = normalized_path_key(&file);
        std::fs::write(&file, "x").expect("write");
        let after_create = normalized_path_key(&file);

        assert_eq!(before_create, after_create);
        assert!(!after_create.starts_with(r"\\?\"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn display_path_strips_canonicalize_prefix() {
        let dir = std::env::temp_dir().join("moyan_display_path_厕所");
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let canon = std::fs::canonicalize(&dir).expect("canonicalize");
        let shown = display_path(&canon);
        #[cfg(windows)]
        assert!(
            canon.to_string_lossy().starts_with(r"\\?\"),
            "Windows canonicalize should yield verbatim path"
        );
        assert!(
            !shown.starts_with(r"\\?\"),
            "display_path must strip verbatim prefix: {shown}"
        );
        assert!(
            shown.contains("厕所"),
            "display_path must keep unicode folder name: {shown}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
