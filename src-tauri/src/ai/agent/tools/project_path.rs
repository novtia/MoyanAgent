//! Resolve model-facing file/folder references within the session project.
//!
//! Tools accept a **project-relative breadcrumb** instead of an absolute path:
//! - `notes.md` — file at project root
//! - `网文测试\第一章.md` — nested file (`/`, `\`, or `>` separators)
//!
//! Absolute paths from user `@` mentions are still accepted for backward
//! compatibility when they point at an existing file under the project.

use std::path::{Path, PathBuf};

use crate::ai::agent::tools::pe_docs;
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
/// basename (or, if the name has no suffix, a unique `.md` / `.txt` document
/// with that title). That convenience is only sound for tools that *read* an
/// existing file — see [`resolve_project_file_strict`] for the write/delete path.
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
        return Err(AppError::Invalid(format!(
            "{tool}: `path` must be non-empty"
        )));
    }

    let as_path = PathBuf::from(raw);
    if as_path.is_absolute() {
        // An absolute path that does not exist (a stale or mistyped prefix,
        // another machine's layout) is not worth failing on when its tail
        // names a file the project really has. Existing paths — including ones
        // outside the project — still go through the strict check below.
        if search == BasenameSearch::Enabled && !as_path.exists() {
            if let Some(found) = resolve_absolute_by_tail(cwd, &as_path, tool)? {
                return Ok(found);
            }
        }
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

    // Read-side convenience only: `深喉验毒` → unique `深喉验毒.md` / `.txt`
    // next to the breadcrumb (or later, anywhere in the tree). Writes must keep
    // the exact name so they do not silently overwrite a suffixed document.
    if search == BasenameSearch::Enabled {
        let last = segments.last().expect("segments non-empty");
        if is_extensionless(last) {
            if let Some(parent) = target.parent() {
                if let Some(found) = resolve_unique_doc_by_stem(parent, last, &root_canon, tool)? {
                    return Ok(found);
                }
            }
        }
        // Breadcrumb that starts with the folders the project root itself ends
        // in (`杂文/我真没想到重生啊/大纲.md` for a project rooted at
        // `…\杂文\我真没想到重生啊`): the leading part is the project, so only
        // the remainder is relative to it. Exact, so it goes before any search.
        if let Some(rest) = strip_root_ancestor_prefix(&root_canon, &segments) {
            if !rest.is_empty() {
                let found = resolve_file_ref(cwd, &rest.join("\\"), tool, search)?;
                if found.is_file() {
                    return Ok(found);
                }
            }
        }
        // The breadcrumb does not exist where it was spelled. Models routinely
        // prefix the path with folders that are *above* the project root (the
        // project is `…\杂文\我真没想到重生啊` and they send
        // `杂文/我真没想到重生啊/大纲.md`), or drop a folder in the middle.
        // Look for the file by its name and keep the candidate whose parent
        // folders agree with the breadcrumb the most; only an unambiguous best
        // match is used.
        if let Some(found) = find_unique_by_path_suffix(&root_canon, &segments, false, 1, tool)? {
            return Ok(found);
        }
    }

    Ok(target)
}

/// If `segments` begins with the last `k` folder names of `root` (compared
/// case-insensitively, longest `k` first), return what follows them.
fn strip_root_ancestor_prefix(root: &Path, segments: &[String]) -> Option<Vec<String>> {
    let names: Vec<String> = root
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().to_lowercase()),
            _ => None,
        })
        .collect();
    let max_k = names.len().min(segments.len());
    (1..=max_k).rev().find_map(|k| {
        let tail = &names[names.len() - k..];
        let head_matches = tail
            .iter()
            .zip(&segments[..k])
            .all(|(a, b)| *a == b.to_lowercase());
        head_matches.then(|| segments[k..].to_vec())
    })
}

/// Map an absolute path that does not exist onto the project by its tail.
fn resolve_absolute_by_tail(cwd: &Path, path: &Path, tool: &str) -> AppResult<Option<PathBuf>> {
    let segments: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    if segments.is_empty() {
        return Ok(None);
    }
    let root = resolve_project_root(cwd)?;
    let root_canon = paths::canonicalize_with_missing_tail(&root).unwrap_or(root);
    // The folders of an absolute path were chosen deliberately, so a bare
    // name coincidence is not enough: require the parent folder to agree too.
    let min_match = segments.len().min(2);
    find_unique_by_path_suffix(&root_canon, &segments, false, min_match, tool)
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
            let canon = std::fs::canonicalize(&as_path)
                .map_err(|e| AppError::Other(format!("{tool}: canonicalize {:?}: {e}", as_path)))?;
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

    if let Some(rest) = strip_root_ancestor_prefix(&root_canon, &segments) {
        let mut t = root_canon.clone();
        for seg in &rest {
            t.push(seg);
        }
        if t.is_dir() {
            return canonicalize_existing(&t, tool);
        }
    }

    // Same leniency as files: `杂文/我真没想到重生啊/第一卷` when the project
    // root already is `…/我真没想到重生啊` finds `第一卷` by its trailing folders.
    // Folders are only listed or searched, never written, so this is safe.
    if let Some(found) = find_unique_by_path_suffix(&root_canon, &segments, true, 1, tool)? {
        return Ok(found);
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
        let canon = std::fs::canonicalize(path)
            .map_err(|e| AppError::Other(format!("{tool}: canonicalize {:?}: {e}", path)))?;
        if let Ok(root) = resolve_project_root(cwd) {
            let root_canon = paths::canonicalize_with_missing_tail(&root).unwrap_or(root);
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
    std::fs::canonicalize(path)
        .map_err(|e| AppError::Other(format!("{tool}: canonicalize {:?}: {e}", path)))
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

fn is_extensionless(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map_or(true, |e| e.is_empty())
}

/// True when `file_name` is the query, or (for an extensionless query) a
/// Markdown / plain-text document whose stem is the query.
fn file_name_matches_query(file_name: &str, query: &str) -> bool {
    if file_name == query {
        return true;
    }
    if !is_extensionless(query) {
        return false;
    }
    let path = Path::new(file_name);
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(pe_docs::is_doc_extension)
        && path.file_stem().and_then(|s| s.to_str()) == Some(query)
}

fn ambiguous_file_error(tool: &str, name: &str, root: &Path, matches: &[PathBuf]) -> AppError {
    let n = matches.len();
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
    AppError::Invalid(format!(
        "{tool}: `{name}` matches {n} files — disambiguate with folder\\{name} or add the suffix: {hint}"
    ))
}

/// If `stem.md` / `stem.txt` / `stem.markdown` exists uniquely under `dir`,
/// return that file. Used when the model passes a document title without a
/// suffix (`深喉验毒` → `深喉验毒.md`).
fn resolve_unique_doc_by_stem(
    dir: &Path,
    stem: &str,
    root: &Path,
    tool: &str,
) -> AppResult<Option<PathBuf>> {
    let mut matches = Vec::new();
    for ext in pe_docs::DOC_EXTENSIONS {
        let candidate = dir.join(format!("{stem}.{ext}"));
        if !candidate.is_file() {
            continue;
        }
        let resolved = std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
        if is_within(root, &resolved) && !matches.iter().any(|p| p == &resolved) {
            matches.push(resolved);
        }
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(Some(matches.remove(0))),
        _ => Err(ambiguous_file_error(tool, stem, root, &matches)),
    }
}

/// Number of trailing path components of `rel` that agree with `segments`
/// (compared from the end). The last component is already known to match by
/// name; the folders above it are compared case-insensitively.
fn suffix_match_len(rel: &Path, segments: &[String]) -> usize {
    let comps: Vec<String> = rel
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    let mut n = 0;
    for (have, want) in comps.iter().rev().zip(segments.iter().rev()) {
        if n > 0 && have.to_lowercase() != want.to_lowercase() {
            break;
        }
        n += 1;
    }
    n
}

/// Find the one file (or, with `want_dir`, folder) under `root` that best fits
/// a breadcrumb which does not exist as written.
///
/// Candidates are everything named like the last segment. They are ranked by
/// how many trailing segments of the breadcrumb their own path ends with, and
/// the best rank wins when it is unique and at least `min_match`. A tie is an
/// error that lists the candidates, so the model can pick instead of guessing.
fn find_unique_by_path_suffix(
    root: &Path,
    segments: &[String],
    want_dir: bool,
    min_match: usize,
    tool: &str,
) -> AppResult<Option<PathBuf>> {
    let Some(name) = segments.last() else {
        return Ok(None);
    };
    let mut candidates = Vec::new();
    let mut budget = MAX_SEARCH_DIRS;
    collect_named(root, name, root, 0, &mut budget, want_dir, &mut candidates)?;

    let scored: Vec<(usize, PathBuf)> = candidates
        .into_iter()
        .map(|p| {
            let n = suffix_match_len(p.strip_prefix(root).unwrap_or(&p), segments);
            (n, p)
        })
        .collect();
    let Some(best) = scored.iter().map(|(n, _)| *n).max() else {
        return Ok(None);
    };
    if best < min_match {
        return Ok(None);
    }
    let mut top: Vec<PathBuf> = scored
        .into_iter()
        .filter(|(n, _)| *n == best)
        .map(|(_, p)| p)
        .collect();
    match top.len() {
        1 => Ok(Some(top.remove(0))),
        _ => Err(ambiguous_file_error(tool, &segments.join("\\"), root, &top)),
    }
}

/// Walk `dir` looking for files (or, with `want_dir`, folders) named `name`.
///
/// Symlinks and Windows junctions are never followed: a link inside the project
/// pointing at `C:\` would otherwise let a bare file name resolve to a file
/// outside the project, and a link pointing back at an ancestor would recurse
/// until the stack overflows. `depth` / `budget` bound pathological trees even
/// when no links are involved, and every match is re-checked against the root
/// because `canonicalize` can still move a path elsewhere.
fn collect_named(
    dir: &Path,
    name: &str,
    root: &Path,
    depth: usize,
    budget: &mut usize,
    want_dir: bool,
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
            if !want_dir
                && path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|n| file_name_matches_query(n, name))
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
            if want_dir
                && path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|n| n.to_lowercase() == name.to_lowercase())
            {
                let resolved = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if is_within(root, &resolved) {
                    out.push(resolved);
                }
            }
            collect_named(&path, name, root, depth + 1, budget, want_dir, out)?;
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
    "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
    "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
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
            assert!(out.starts_with('_'), "`{raw}` must be defused, got `{out}`");
        }
        for raw in ["console.md", "context.txt", "communication.md", "notes.md"] {
            assert_eq!(sanitize_segment(raw), raw, "`{raw}` is an ordinary name");
        }
    }

    #[test]
    fn containment_compares_on_a_separator_boundary() {
        let root = std::env::temp_dir().join(format!("moyan_within_{}", std::process::id()));
        let sibling = std::env::temp_dir().join(format!("moyan_within_{}-old", std::process::id()));
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

    fn unique_temp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "moyan_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp");
        dir
    }

    /// `深喉验毒` must resolve to the unique `深喉验毒.md` at the project root.
    #[test]
    fn extensionless_title_resolves_to_unique_doc_suffix() {
        let root = unique_temp("extless_root");
        std::fs::write(root.join("深喉验毒.md"), "body").expect("seed");

        let found = resolve_project_file(&root, "深喉验毒", "Read").expect("resolve");
        assert_eq!(
            found.file_name().and_then(|s| s.to_str()),
            Some("深喉验毒.md")
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Nested breadcrumb without a suffix: `drafts\深喉验毒` → `drafts\深喉验毒.txt`.
    #[test]
    fn extensionless_nested_breadcrumb_resolves_beside_the_folder() {
        let root = unique_temp("extless_nested");
        std::fs::create_dir_all(root.join("drafts")).expect("mkdir");
        std::fs::write(root.join("drafts").join("深喉验毒.txt"), "body").expect("seed");

        let found = resolve_project_file(&root, r"drafts\深喉验毒", "Read").expect("resolve");
        assert!(
            found.ends_with(Path::new("drafts").join("深喉验毒.txt")),
            "got {found:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Bare title with no root copy still finds the unique nested document.
    #[test]
    fn extensionless_title_finds_unique_nested_doc() {
        let root = unique_temp("extless_buried");
        std::fs::create_dir_all(root.join("chapters")).expect("mkdir");
        std::fs::write(root.join("chapters").join("深喉验毒.md"), "body").expect("seed");

        let found = resolve_project_file(&root, "深喉验毒", "Read").expect("resolve");
        assert!(
            found.ends_with(Path::new("chapters").join("深喉验毒.md")),
            "got {found:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn extensionless_title_is_ambiguous_when_md_and_txt_both_exist() {
        let root = unique_temp("extless_ambig");
        std::fs::write(root.join("notes.md"), "md").expect("seed md");
        std::fs::write(root.join("notes.txt"), "txt").expect("seed txt");

        let err = resolve_project_file(&root, "notes", "Read").expect_err("ambiguous");
        let msg = err.to_string();
        assert!(msg.contains("matches 2 files"), "{msg}");
        assert!(
            msg.contains("notes.md") && msg.contains("notes.txt"),
            "{msg}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The reported failure: project root is `…/杂文/我真没想到重生啊` and the
    /// model prefixed the breadcrumb with the folders above it.
    #[test]
    fn breadcrumb_prefixed_with_the_roots_own_folders_resolves() {
        let base = unique_temp("ancestor_prefix");
        let root = base.join("杂文").join("我真没想到重生啊");
        std::fs::create_dir_all(root.join("第一卷")).expect("mkdir");
        std::fs::write(root.join("大纲.md"), "outline").expect("seed");
        // A same-named file deeper down must not make the root copy ambiguous.
        std::fs::write(root.join("第一卷").join("大纲.md"), "volume outline").expect("seed");

        let found = resolve_project_file(&root, "杂文/我真没想到重生啊/大纲.md", "Read")
            .expect("resolve");
        assert_eq!(std::fs::read_to_string(&found).unwrap(), "outline");

        let found = resolve_project_file(&root, "我真没想到重生啊/大纲.md", "Read").expect("resolve");
        assert_eq!(std::fs::read_to_string(&found).unwrap(), "outline");

        let dir = resolve_project_dir(&root, Some("杂文/我真没想到重生啊/第一卷"), "ListFiles")
            .expect("dir");
        assert!(dir.ends_with("第一卷"), "{dir:?}");
        let dir =
            resolve_project_dir(&root, Some("杂文/我真没想到重生啊"), "ListFiles").expect("root");
        assert!(dir.ends_with("我真没想到重生啊"), "{dir:?}");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// A folder dropped from (or added to) the middle of the breadcrumb still
    /// finds the one file with that name; two candidates stay an error.
    #[test]
    fn breadcrumb_with_a_wrong_folder_finds_the_best_match() {
        let root = unique_temp("suffix_rank");
        std::fs::create_dir_all(root.join("第一卷").join("设定")).expect("mkdir");
        std::fs::create_dir_all(root.join("第二卷")).expect("mkdir");
        std::fs::write(root.join("第一卷").join("设定").join("人物.md"), "a").expect("seed");
        std::fs::write(root.join("第二卷").join("人物.md"), "b").expect("seed");

        // Two files share the name; the breadcrumb's folder decides.
        let found = resolve_project_file(&root, "草稿/设定/人物.md", "Read").expect("resolve");
        assert!(found.ends_with(Path::new("设定").join("人物.md")), "{found:?}");

        // Nothing in the breadcrumb tells them apart → list the candidates.
        let err = resolve_project_file(&root, "草稿/人物.md", "Read").expect_err("ambiguous");
        assert!(err.to_string().contains("matches 2 files"), "{err}");

        // Writes never retarget.
        let strict =
            resolve_project_file_strict(&root, "草稿/人物.md", "Write").expect("strict");
        assert!(!strict.is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A stale absolute prefix maps onto the project by its tail, but only when
    /// the parent folder agrees too.
    #[test]
    fn nonexistent_absolute_path_maps_onto_the_project_by_its_tail() {
        let root = unique_temp("abs_tail");
        std::fs::create_dir_all(root.join("杂文")).expect("mkdir");
        std::fs::write(root.join("杂文").join("大纲.md"), "x").expect("seed");

        let stale = Path::new(if cfg!(windows) { r"Z:\old\box" } else { "/old/box" })
            .join("杂文")
            .join("大纲.md");
        let found = resolve_project_file(&root, stale.to_str().unwrap(), "Read").expect("resolve");
        assert!(found.ends_with(Path::new("杂文").join("大纲.md")), "{found:?}");

        let wrong_parent = Path::new(if cfg!(windows) { r"Z:\old" } else { "/old" })
            .join("别处")
            .join("大纲.md");
        assert!(
            resolve_project_file(&root, wrong_parent.to_str().unwrap(), "Read").is_err(),
            "a bare name coincidence is not enough for an absolute path"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn suffixed_query_does_not_match_a_sibling_with_another_extension() {
        let root = unique_temp("extless_exact");
        std::fs::write(root.join("notes.txt"), "txt").expect("seed");

        let found = resolve_project_file(&root, "notes.md", "Read").expect("exact path");
        assert_eq!(
            found.file_name().and_then(|s| s.to_str()),
            Some("notes.md"),
            "a `.md` query must not silently retarget at `.txt`, got {found:?}"
        );
        assert!(!found.is_file());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn strict_resolution_does_not_append_a_doc_suffix() {
        let root = unique_temp("extless_strict");
        std::fs::write(root.join("notes.md"), "md").expect("seed");

        let strict = resolve_project_file_strict(&root, "notes", "Write").expect("strict resolve");
        assert_eq!(strict.file_name().and_then(|s| s.to_str()), Some("notes"));
        assert!(
            !strict.as_os_str().to_string_lossy().ends_with("notes.md"),
            "writes must keep the exact name, got {strict:?}"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
