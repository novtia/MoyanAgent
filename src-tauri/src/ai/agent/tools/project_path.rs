//! Resolve model-facing file/folder references within the session project.
//!
//! Tools accept a **project-relative breadcrumb** instead of an absolute path:
//! - `notes.md` — file at project root
//! - `网文测试\第一章.md` — nested file (`/`, `\`, or `>` separators)
//!
//! Absolute paths from user `@` mentions are still accepted for backward
//! compatibility when they point at an existing file under the project.

use std::path::{Path, PathBuf};

use crate::data::paths;
use crate::error::{AppError, AppResult};

/// Path string for tool JSON / UI (strips Windows `\\?\` verbatim prefixes).
pub fn display_path(path: &Path) -> String {
    paths::display_path(path)
}

pub const FILE_REF_DESC: &str = "Project file: file name at the project root (e.g. `notes.md`), \
    or a folder\\file breadcrumb for nested files (e.g. `drafts\\chapter-01.md`). \
    Do not pass absolute paths.";

pub const DIR_REF_DESC: &str = "Project folder as a breadcrumb (e.g. `notes`, `chapters\\01`). \
    Omit or leave empty for the project root.";

/// Session working directory when absolute, otherwise the app's user root
/// (`Documents/MoYanAgent`).
///
/// The fallback must come from [`paths::user_moyan_root`] and not a
/// locally-spelled constant: the rollback writer only accepts paths under
/// `MoYanAgent` / `MoYanAgent/Project`, so a differently-cased duplicate root
/// would make documents created in a project-less session impossible to roll
/// back (and, on a case-sensitive volume, land in a second folder entirely).
pub fn resolve_project_root(cwd: &Path) -> AppResult<PathBuf> {
    if !cwd.as_os_str().is_empty() && cwd.is_absolute() {
        return Ok(cwd.to_path_buf());
    }
    paths::user_moyan_root()
}

/// Depth cap for the basename fallback search. Deep project trees are fine;
/// this only stops pathological recursion.
const MAX_SEARCH_DEPTH: usize = 24;

/// Upper bound on directories visited by one basename search.
const MAX_SEARCH_DIRS: usize = 20_000;

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
///
/// When the breadcrumb is a bare file name that does not exist at the project
/// root, the whole project tree is searched for a unique file with that
/// basename. That convenience is only sound for tools that *read* an existing
/// file — see [`resolve_project_file_strict`] for the write/delete path.
pub fn resolve_project_file(cwd: &Path, raw: &str, tool: &str) -> AppResult<PathBuf> {
    resolve_file_ref(cwd, raw, tool, BasenameSearch::Enabled)
}

/// Resolve a project file reference **without** the whole-tree basename search.
///
/// Tools that create, overwrite or delete must land exactly where the
/// breadcrumb points: retargeting `notes.md` at `chapters/notes.md` because the
/// root copy does not exist yet would overwrite or delete an unrelated file
/// that merely shares a name.
pub fn resolve_project_file_strict(cwd: &Path, raw: &str, tool: &str) -> AppResult<PathBuf> {
    resolve_file_ref(cwd, raw, tool, BasenameSearch::Disabled)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BasenameSearch {
    Enabled,
    Disabled,
}

fn resolve_file_ref(
    cwd: &Path,
    raw: &str,
    tool: &str,
    search: BasenameSearch,
) -> AppResult<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::Invalid(format!("{tool}: `path` must be non-empty")));
    }

    let as_path = PathBuf::from(raw);
    if as_path.is_absolute() {
        return resolve_absolute_compat(cwd, &as_path, tool);
    }

    let root = resolve_project_root(cwd)?;
    let root_canon = paths::canonicalize_with_missing_tail(&root).unwrap_or_else(|_| root.clone());

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

    if search == BasenameSearch::Enabled && segments.len() == 1 {
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
            let root_canon =
                paths::canonicalize_with_missing_tail(&root).unwrap_or(root);
            if !is_within(&root_canon, &canon) {
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

pub use crate::data::paths::is_within;

fn ensure_within_project_root(project_root: &Path, target: &Path, tool: &str) -> AppResult<()> {
    if is_within(project_root, target) {
        return Ok(());
    }
    Err(AppError::Invalid(format!(
        "{tool}: path resolves outside the project root"
    )))
}

fn find_unique_file_by_basename(
    root: &Path,
    name: &str,
    tool: &str,
) -> AppResult<Option<PathBuf>> {
    let mut matches = Vec::new();
    let mut budget = MAX_SEARCH_DIRS;
    collect_files_named(root, name, root, 0, &mut budget, &mut matches)?;
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.remove(0))),
        n => {
            let rel: Vec<String> = matches
                .iter()
                .take(5)
                .filter_map(|p| p.strip_prefix(root).ok())
                .map(|p| p.to_string_lossy().into_owned())
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

/// Walk `dir` looking for files named `name`.
///
/// Symlinks and Windows junctions are never followed: a link inside the project
/// pointing at `C:\` would otherwise let a bare file name resolve to a file
/// outside the project, and a link pointing back at an ancestor would recurse
/// until the stack overflows. `depth` / `budget` bound pathological trees even
/// when no links are involved, and every match is re-checked against the root
/// because `canonicalize` can still move a path elsewhere.
fn collect_files_named(
    dir: &Path,
    name: &str,
    root: &Path,
    depth: usize,
    budget: &mut usize,
    out: &mut Vec<PathBuf>,
) -> AppResult<()> {
    if depth > MAX_SEARCH_DEPTH || *budget == 0 {
        return Ok(());
    }
    *budget -= 1;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(()),
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_file() {
            if path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n == name)
            {
                let resolved = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if is_within(root, &resolved) {
                    out.push(resolved);
                }
            }
        } else if file_type.is_dir() {
            // `file_type().is_dir()` is false for a Unix symlink-to-directory
            // but true for a Windows junction, so verify the entry is not a
            // reparse point before descending.
            if is_reparse_point(&path) {
                continue;
            }
            collect_files_named(&path, name, root, depth + 1, budget, out)?;
        }
    }
    Ok(())
}

/// True when `path` is a Windows reparse point (junction / directory symlink).
/// Always false elsewhere — `file_type().is_symlink()` already covers Unix.
#[cfg(windows)]
fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    std::fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(true)
}

#[cfg(not(windows))]
fn is_reparse_point(_path: &Path) -> bool {
    false
}

/// Names Windows resolves to character devices rather than files. Opening one
/// (even via `exists()`) can block forever — `CON` waits on console input — so
/// they must never survive as a path segment.
const WINDOWS_DEVICE_NAMES: &[&str] = &[
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

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

    // A device name is reserved with *any* extension (`CON`, `con.txt`).
    let stem = s
        .split('.')
        .next()
        .unwrap_or(&s)
        .trim()
        .to_ascii_lowercase();
    if WINDOWS_DEVICE_NAMES.contains(&stem.as_str()) {
        return format!("_{s}");
    }
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

    /// Opening `CON` blocks on console input, which would hang a tool that only
    /// meant to stat a file.
    #[test]
    fn windows_device_names_never_survive_as_a_segment() {
        for raw in ["CON", "con", "nul", "com1", "LPT1", "con.txt", "aux.md"] {
            let out = sanitize_segment(raw);
            assert!(
                out.starts_with('_'),
                "`{raw}` must be defused, got `{out}`"
            );
        }
        for raw in ["console.md", "context.txt", "communication.md", "notes.md"] {
            assert_eq!(sanitize_segment(raw), raw, "`{raw}` is an ordinary name");
        }
    }

    #[test]
    fn containment_compares_on_a_separator_boundary() {
        let root = std::env::temp_dir().join(format!("moyan_within_{}", std::process::id()));
        let sibling = std::env::temp_dir()
            .join(format!("moyan_within_{}-old", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
        std::fs::create_dir_all(&root).expect("create root");
        std::fs::create_dir_all(&sibling).expect("create sibling");

        assert!(is_within(&root, &root), "root contains itself");
        assert!(is_within(&root, &root.join("chapters").join("01.md")));
        assert!(
            !is_within(&root, &sibling),
            "a name-prefixed sibling is not a child"
        );
        assert!(!is_within(&root, &sibling.join("leak.md")));

        // A path whose tail does not exist yet must still resolve correctly:
        // on Windows the root canonicalizes to `\\?\C:\…` while the target
        // does not, which a raw `starts_with` would reject.
        assert!(is_within(&root, &root.join("not-created-yet").join("a.md")));

        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&sibling);
    }

    /// Write / Delete must land exactly where the breadcrumb points, or a bare
    /// `notes.md` could overwrite `chapters/notes.md`.
    #[test]
    fn strict_resolution_does_not_retarget_a_bare_name_at_a_nested_file() {
        let root = std::env::temp_dir().join(format!("moyan_strict_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("chapters")).expect("create nested dir");
        std::fs::write(root.join("chapters").join("notes.md"), "nested").expect("seed file");

        let lenient = resolve_project_file(&root, "notes.md", "Read").expect("lenient resolve");
        assert!(
            lenient.ends_with(Path::new("chapters").join("notes.md")),
            "reads may follow the unique nested match, got {lenient:?}"
        );

        let strict =
            resolve_project_file_strict(&root, "notes.md", "Write").expect("strict resolve");
        assert_eq!(
            strict.file_name().and_then(|s| s.to_str()),
            Some("notes.md")
        );
        assert!(
            !strict.ends_with(Path::new("chapters").join("notes.md")),
            "writes must stay at the root, got {strict:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
