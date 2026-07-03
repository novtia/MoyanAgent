//! Resolve model-facing file/folder references within the session project.
//!
//! Tools accept a **project-relative breadcrumb** instead of an absolute path:
//! - `notes.md` — file at project root
//! - `网文测试\第一章.md` — nested file (`/`, `\`, or `>` separators)
//!
//! Absolute paths from user `@` mentions are still accepted for backward
//! compatibility when they point at an existing file under the project.

use std::path::{Path, PathBuf};

use crate::error::{AppError, AppResult};

pub const FILE_REF_DESC: &str = "Project file: file name at the project root (e.g. `notes.md`), \
    or a folder\\file breadcrumb for nested files (e.g. `drafts\\chapter-01.md`). \
    Do not pass absolute paths.";

pub const DIR_REF_DESC: &str = "Project folder as a breadcrumb (e.g. `notes`, `chapters\\01`). \
    Omit or leave empty for the project root.";

const FALLBACK_DIR_NAME: &str = "moyanagent";

/// Session working directory when absolute, otherwise `~/Documents/moyanagent`.
pub fn resolve_project_root(cwd: &Path) -> AppResult<PathBuf> {
    if !cwd.as_os_str().is_empty() && cwd.is_absolute() {
        return Ok(cwd.to_path_buf());
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| AppError::Other("cannot resolve user home directory".into()))?;
    Ok(home.join("Documents").join(FALLBACK_DIR_NAME))
}

/// Split a breadcrumb string into sanitized path segments (`/`, `\`, or `>`).
pub fn parse_breadcrumb_segments(raw: &str, tool: &str) -> AppResult<Vec<String>> {
    let mut segments = Vec::new();
    for part in raw.split(['/', '\\']) {
        for crumb in part.split('>') {
            let trimmed = crumb.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed == ".." || trimmed == "." {
                return Err(AppError::Invalid(format!(
                    "{tool}: path must stay inside the project (invalid segment `{trimmed}`)"
                )));
            }
            let name = sanitize_segment(trimmed);
            if name.is_empty() {
                continue;
            }
            segments.push(name);
        }
    }
    Ok(segments)
}

/// Resolve a project file reference to an absolute path.
pub fn resolve_project_file(cwd: &Path, raw: &str, tool: &str) -> AppResult<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::Invalid(format!("{tool}: `path` must be non-empty")));
    }

    let as_path = PathBuf::from(raw);
    if as_path.is_absolute() {
        return resolve_absolute_compat(cwd, &as_path, tool);
    }

    let root = resolve_project_root(cwd)?;
    let root_canon = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());

    let segments = parse_breadcrumb_segments(raw, tool)?;
    if segments.is_empty() {
        return Err(AppError::Invalid(format!(
            "{tool}: `path` must be a file name or folder\\file breadcrumb"
        )));
    }

    let mut target = root_canon.clone();
    for seg in &segments {
        target.push(seg);
    }
    ensure_within_project_root(&root_canon, &target, tool)?;

    if target.is_file() {
        return canonicalize_existing(&target, tool);
    }

    if segments.len() == 1 {
        if let Some(found) = find_unique_file_by_basename(&root_canon, &segments[0], tool)? {
            return Ok(found);
        }
    }

    Ok(target)
}

/// Resolve a project folder reference (empty → project root).
pub fn resolve_project_dir(cwd: &Path, raw: Option<&str>, tool: &str) -> AppResult<PathBuf> {
    let root = resolve_project_root(cwd)?;
    let root_canon = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());

    let raw = raw.unwrap_or("").trim();
    if raw.is_empty() {
        return Ok(root_canon);
    }

    let as_path = PathBuf::from(raw);
    if as_path.is_absolute() {
        if as_path.is_dir() {
            let canon = std::fs::canonicalize(&as_path).map_err(|e| {
                AppError::Other(format!("{tool}: canonicalize {:?}: {e}", as_path))
            })?;
            ensure_within_project_root(&root_canon, &canon, tool)?;
            return Ok(canon);
        }
        return Err(AppError::Invalid(format!(
            "{tool}: not a directory: {}",
            as_path.display()
        )));
    }

    let segments = parse_breadcrumb_segments(raw, tool)?;
    if segments.is_empty() {
        return Ok(root_canon);
    }

    let mut target = root_canon.clone();
    for seg in &segments {
        target.push(seg);
    }
    ensure_within_project_root(&root_canon, &target, tool)?;

    if target.is_dir() {
        return canonicalize_existing(&target, tool);
    }

    Err(AppError::Invalid(format!(
        "{tool}: directory not found: `{raw}` (relative to project root)"
    )))
}

/// Grep accepts either a file or directory reference.
pub fn resolve_project_file_or_dir(cwd: &Path, raw: &str, tool: &str) -> AppResult<PathBuf> {
    let file = resolve_project_file(cwd, raw, tool)?;
    if file.is_file() {
        return Ok(file);
    }
    if file.is_dir() {
        return Ok(file);
    }

    resolve_project_dir(cwd, Some(raw), tool)
}

fn resolve_absolute_compat(cwd: &Path, path: &Path, tool: &str) -> AppResult<PathBuf> {
    if path.exists() {
        let canon = std::fs::canonicalize(path).map_err(|e| {
            AppError::Other(format!("{tool}: canonicalize {:?}: {e}", path))
        })?;
        if let Ok(root) = resolve_project_root(cwd) {
            let root_canon = std::fs::canonicalize(&root).unwrap_or(root);
            if !canon.starts_with(&root_canon) {
                return Err(AppError::Invalid(format!(
                    "{tool}: path is outside the project root — use a file name or folder\\file breadcrumb instead"
                )));
            }
        }
        return Ok(canon);
    }
    Err(AppError::Invalid(format!(
        "{tool}: file not found: `{}` — use a file name or folder\\file breadcrumb within the project",
        path.display()
    )))
}

fn canonicalize_existing(path: &Path, tool: &str) -> AppResult<PathBuf> {
    std::fs::canonicalize(path).map_err(|e| {
        AppError::Other(format!("{tool}: canonicalize {:?}: {e}", path))
    })
}

fn ensure_within_project_root(project_root: &Path, target: &Path, tool: &str) -> AppResult<()> {
    if target.exists() {
        let canon = std::fs::canonicalize(target).map_err(|e| {
            AppError::Other(format!("{tool}: canonicalize {:?}: {e}", target))
        })?;
        if !canon.starts_with(project_root) {
            return Err(AppError::Invalid(format!(
                "{tool}: path resolves outside the project root"
            )));
        }
        return Ok(());
    }

    let mut probe = target.to_path_buf();
    while !probe.exists() {
        if probe == project_root {
            break;
        }
        probe = probe
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| project_root.to_path_buf());
    }
    if probe.exists() {
        let canon = std::fs::canonicalize(&probe).map_err(|e| {
            AppError::Other(format!("{tool}: canonicalize {:?}: {e}", probe))
        })?;
        if !canon.starts_with(project_root) {
            return Err(AppError::Invalid(format!(
                "{tool}: path resolves outside the project root"
            )));
        }
    } else if !target.starts_with(project_root) {
        return Err(AppError::Invalid(format!(
            "{tool}: path resolves outside the project root"
        )));
    }
    Ok(())
}

fn find_unique_file_by_basename(
    root: &Path,
    name: &str,
    tool: &str,
) -> AppResult<Option<PathBuf>> {
    let mut matches = Vec::new();
    collect_files_named(root, name, root, &mut matches)?;
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.remove(0))),
        n => {
            let rel: Vec<String> = matches
                .iter()
                .take(5)
                .filter_map(|p| p.strip_prefix(root).ok())
                .map(|p| p.to_string_lossy().replace('\\', "\\"))
                .collect();
            let hint = if n > 5 {
                format!("{} … (+{} more)", rel.join(", "), n - 5)
            } else {
                rel.join(", ")
            };
            Err(AppError::Invalid(format!(
                "{tool}: `{name}` matches {n} files — disambiguate with folder\\{name}: {hint}"
            )))
        }
    }
}

fn collect_files_named(
    dir: &Path,
    name: &str,
    root: &Path,
    out: &mut Vec<PathBuf>,
) -> AppResult<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n == name)
            {
                if let Ok(canon) = std::fs::canonicalize(&path) {
                    out.push(canon);
                } else {
                    out.push(path);
                }
            }
        } else if path.is_dir() {
            collect_files_named(&path, name, root, out)?;
        }
    }
    Ok(())
}

fn sanitize_segment(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let s = s.trim().trim_end_matches('.').trim().to_string();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_breadcrumb_splits_separators() {
        assert_eq!(
            parse_breadcrumb_segments("a\\b.md", "T").unwrap(),
            vec!["a".to_string(), "b.md".to_string()]
        );
        assert_eq!(
            parse_breadcrumb_segments("a/b.md", "T").unwrap(),
            vec!["a".to_string(), "b.md".to_string()]
        );
        assert_eq!(
            parse_breadcrumb_segments("a > b.md", "T").unwrap(),
            vec!["a".to_string(), "b.md".to_string()]
        );
    }

    #[test]
    fn parse_breadcrumb_preserves_spaces_in_filename() {
        assert_eq!(
            parse_breadcrumb_segments("网文测试\\第一章 穿越.md", "T").unwrap(),
            vec!["网文测试".to_string(), "第一章 穿越.md".to_string()]
        );
    }

    #[test]
    fn parse_breadcrumb_rejects_parent_dir() {
        assert!(parse_breadcrumb_segments("../x", "T").is_err());
    }
}
