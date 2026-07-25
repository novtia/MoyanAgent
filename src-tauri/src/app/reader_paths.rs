use std::path::PathBuf;

use crate::data::db;
use crate::data::{paths, project, session};
use crate::error::{AppError, AppResult};

pub(crate) fn session_project_cwd(
    conn: &db::DbConn,
    session_id: &str,
) -> Option<std::path::PathBuf> {
    let sess = session::get(conn, session_id).ok()?;
    let project_id = sess.project_id?;
    let proj = project::get(conn, &project_id).ok()?;
    let path_str = proj.path.filter(|p| !p.trim().is_empty())?;
    Some(std::path::PathBuf::from(path_str))
}

/// Allowed roots for user-initiated writes from the reader panel.
pub(crate) fn reader_write_roots(session_cwd: Option<&std::path::Path>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(cwd) = session_cwd {
        if cwd.is_absolute() {
            roots.push(cwd.to_path_buf());
        }
    }
    if let Ok(root) = paths::user_moyan_root() {
        roots.push(root);
    }
    if let Ok(root) = paths::blank_projects_root() {
        roots.push(root);
    }
    roots
}

pub(crate) fn path_under_root(path: &std::path::Path, root: &std::path::Path) -> bool {
    path.starts_with(root)
}

/// Canonicalize an existing path, or its nearest existing ancestor when the
/// final file/folders have not been created yet.
///
/// On Windows, `canonicalize` returns a verbatim `\\?\` path. Comparing that
/// canonical root with an untouched non-existent target makes a valid child
/// appear to be outside the root. Rebuilding the missing tail on top of the
/// canonical ancestor keeps both sides in the same path representation.
pub(crate) fn canonicalize_reader_path(path: &std::path::Path) -> AppResult<PathBuf> {
    if path
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(AppError::Invalid(
            "write_project_file: path traversal is not allowed".into(),
        ));
    }

    let mut ancestor = path;
    let mut missing = Vec::new();

    while !ancestor.exists() {
        let segment = ancestor.file_name().ok_or_else(|| {
            AppError::Invalid(format!(
                "write_project_file: cannot resolve path {}",
                path.display()
            ))
        })?;
        if segment == "." || segment == ".." {
            return Err(AppError::Invalid(
                "write_project_file: path traversal is not allowed".into(),
            ));
        }
        missing.push(segment.to_os_string());
        ancestor = ancestor.parent().ok_or_else(|| {
            AppError::Invalid(format!(
                "write_project_file: cannot resolve path {}",
                path.display()
            ))
        })?;
    }

    let mut resolved = std::fs::canonicalize(ancestor).map_err(|e| {
        AppError::Other(format!(
            "write_project_file: canonicalize {}: {e}",
            ancestor.display()
        ))
    })?;
    for segment in missing.iter().rev() {
        resolved.push(segment);
    }
    Ok(resolved)
}

pub(crate) fn validate_reader_write_path(
    file_path: &std::path::Path,
    session_cwd: Option<&std::path::Path>,
) -> AppResult<PathBuf> {
    let canonical = canonicalize_reader_path(file_path)?;
    let roots = reader_write_roots(session_cwd);
    if roots.is_empty() {
        return Err(AppError::Invalid(
            "write_project_file: no allowed write directory for this session".into(),
        ));
    }
    for root in &roots {
        let Ok(root_canon) = canonicalize_reader_path(root) else {
            continue;
        };
        if path_under_root(&canonical, &root_canon) {
            return Ok(canonical);
        }
    }
    Err(AppError::Invalid(format!(
        "write_project_file: path {:?} is outside the project or allowed documents folder",
        file_path.display()
    )))
}

#[cfg(test)]
mod reader_write_path_tests {
    use super::{canonicalize_reader_path, validate_reader_write_path};

    pub(crate) fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("atelier-reader-path-{name}-{}", std::process::id()))
    }

    #[test]
    pub(crate) fn canonicalizes_nonexistent_child_using_existing_ancestor() {
        let root = temp_root("new-child");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let target = root.join("杂文").join("新建文件.txt");
        let canonical_root = std::fs::canonicalize(&root).unwrap();
        let resolved = validate_reader_write_path(&target, Some(&root)).unwrap();

        assert_eq!(resolved, canonical_root.join("杂文").join("新建文件.txt"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    pub(crate) fn rejects_parent_traversal_in_nonexistent_tail() {
        let root = temp_root("traversal");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let target = root.join("missing").join("..").join("outside.txt");
        assert!(canonicalize_reader_path(&target).is_err());
        let _ = std::fs::remove_dir_all(root);
    }
}
