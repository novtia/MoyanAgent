//! `CreateDoc` — a deliberately tiny document-authoring tool.
//!
//! Unlike [`crate::ai::agent::tools::edit::FileWriteTool`] (which demands an
//! absolute path and a prior read receipt), `CreateDoc` asks the model for
//! only three things: a `title`, the `content`, and a `doc_type` (`md` or
//! `txt`), plus an optional `folder` breadcrumb within the project. The file
//! is written under the session's project working directory
//! (`ToolUseContext::cwd`), falling back to `~/Documents/moyanagent` when the
//! session has no project path. Writes are always confined to that project
//! root — `folder` cannot escape it. The successful result echoes the full text
//! plus word-count stats so the UI can open the freshly created document in
//! the reader panel with a single click.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{FileOp, FileSnapshotStore};
use crate::ai::agent::tools::text_decode::normalize_tool_string;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "CreateDoc";

/// Sub-directory used when no project working directory is available.
const FALLBACK_DIR_NAME: &str = "moyanagent";

#[derive(Clone)]
pub struct CreateDocTool {
    spec: ToolSpec,
    snapshots: Arc<FileSnapshotStore>,
}

impl CreateDocTool {
    pub fn new(snapshots: Arc<FileSnapshotStore>) -> Self {
        Self {
            snapshots,
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Create a text document from a title, its content, and a type. \
                    The file is saved inside the current project folder only \
                    (or the user's Documents folder when no project is set). \
                    Optionally pass `folder` to place it in a subfolder: a single \
                    name like `notes`, or a nested breadcrumb like `chapters/01` \
                    or `chapters > 01`. \
                    `doc_type` is `md` (Markdown) or `txt` (plain text). \
                    Prefer this over Write for authoring new documents — you only \
                    supply the title, the content, the type, and optionally a folder."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Document title; also used as the file name."
                        },
                        "content": {
                            "type": "string",
                            "description": "Full document text."
                        },
                        "doc_type": {
                            "type": "string",
                            "enum": ["md", "txt"],
                            "description": "Document type: `md` for Markdown, `txt` for plain text."
                        },
                        "folder": {
                            "type": "string",
                            "description": "Optional subfolder within the project. \
                                Single folder name or breadcrumb, e.g. `notes`, \
                                `网文测试\\草稿`, `chapters\\01`. Omit to save at project root."
                        }
                    },
                    "required": ["title", "content", "doc_type"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

impl Tool for CreateDocTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let title = input
            .get("title")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid(format!("{TOOL_NAME}: `title` must be a string")))?;
        if title.trim().is_empty() {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `title` must be non-empty"
            )));
        }
        input
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid(format!("{TOOL_NAME}: `content` must be a string")))?;
        input.get("doc_type").and_then(Value::as_str).ok_or_else(|| {
            AppError::Invalid(format!("{TOOL_NAME}: `doc_type` must be a string"))
        })?;
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let title = invocation
                .input
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string();
            let content = normalize_tool_string(
                invocation
                    .input
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            let doc_type = invocation
                .input
                .get("doc_type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let folder = invocation.input.get("folder").and_then(Value::as_str);

            let ext = match doc_type.trim().to_lowercase().as_str() {
                "md" | "markdown" => "md",
                "txt" | "text" | "plain" => "txt",
                other => {
                    return Ok(ToolResult::error(format!(
                        "{TOOL_NAME}: unsupported doc_type `{other}` (use `md` or `txt`)"
                    )));
                }
            };

            let dir = resolve_output_dir(&invocation.context.cwd, folder)?;
            std::fs::create_dir_all(&dir)
                .map_err(|e| AppError::Other(format!("{TOOL_NAME}: mkdir {:?}: {e}", dir)))?;

            let file_name = format!("{}.{ext}", sanitize_file_name(&title));
            let path = dir.join(&file_name);
            let created = !path.exists();

            // Snapshot the pre-image before writing so the document can be
            // rolled back (deleted when created, restored when overwritten).
            let op = if created { FileOp::Create } else { FileOp::Update };
            self.snapshots
                .record_before(invocation.context.session_id.as_deref(), &path, op);

            std::fs::write(&path, content.as_bytes())
                .map_err(|e| AppError::Other(format!("{TOOL_NAME}: write {:?}: {e}", path)))?;

            // Canonicalize for a stable, absolute path; fall back to the
            // joined path if canonicalization fails for any reason.
            let canonical = std::fs::canonicalize(&path).unwrap_or(path);

            // Stamp a read receipt so a follow-up `Edit` on this file works
            // without a separate `Read` round-trip.
            if let Ok(mut s) = invocation.context.read_file_state.lock() {
                s.insert(canonical.clone());
            }

            let chars = content.chars().filter(|c| !c.is_whitespace()).count();
            let lines = content.lines().count();

            Ok(ToolResult::ok(json!({
                "path": canonical.to_string_lossy(),
                "title": title,
                "doc_type": ext,
                "folder": folder.map(str::trim).filter(|s| !s.is_empty()),
                "bytes": content.len(),
                "chars": chars,
                "lines": lines,
                "created": created,
                "text": content,
            })))
        })
    }
}

/// Resolve the project root: session working directory when absolute, otherwise
/// `~/Documents/<FALLBACK_DIR_NAME>`.
fn resolve_project_root(cwd: &Path) -> AppResult<PathBuf> {
    if !cwd.as_os_str().is_empty() && cwd.is_absolute() {
        return Ok(cwd.to_path_buf());
    }
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .ok_or_else(|| {
            AppError::Other(format!("{TOOL_NAME}: cannot resolve user home directory"))
        })?;
    Ok(home.join("Documents").join(FALLBACK_DIR_NAME))
}

/// Resolve the output directory: project root, optionally extended by a
/// breadcrumb `folder` path that must stay inside the project.
fn resolve_output_dir(cwd: &Path, folder: Option<&str>) -> AppResult<PathBuf> {
    let root = resolve_project_root(cwd)?;
    let root_canon = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());

    let raw = folder.unwrap_or("").trim();
    if raw.is_empty() {
        return Ok(root_canon);
    }

    let segments = parse_folder_breadcrumb(raw)?;
    let mut dir = root_canon.clone();
    for seg in &segments {
        dir.push(seg);
    }

    ensure_within_project_root(&root_canon, &dir)
}

/// Split a breadcrumb folder string into sanitized path segments.
/// Accepts `/`, `\`, or `>` as separators (matching the file explorer crumbs).
fn parse_folder_breadcrumb(raw: &str) -> AppResult<Vec<String>> {
    let mut segments = Vec::new();
    for part in raw.split(['/', '\\']) {
        for crumb in part.split('>') {
            let trimmed = crumb.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed == ".." || trimmed == "." {
                return Err(AppError::Invalid(format!(
                    "{TOOL_NAME}: `folder` must stay inside the project (invalid segment `{trimmed}`)"
                )));
            }
            let name = sanitize_folder_segment(trimmed);
            if name.is_empty() {
                continue;
            }
            segments.push(name);
        }
    }
    if segments.is_empty() {
        return Err(AppError::Invalid(format!(
            "{TOOL_NAME}: `folder` must contain at least one folder name"
        )));
    }
    Ok(segments)
}

/// Ensure `target` resolves to a path under `project_root` (blocks `..` and symlinks).
fn ensure_within_project_root(project_root: &Path, target: &Path) -> AppResult<PathBuf> {
    if target.exists() {
        let canon = std::fs::canonicalize(target).map_err(|e| {
            AppError::Other(format!(
                "{TOOL_NAME}: canonicalize {:?}: {e}",
                target
            ))
        })?;
        if !canon.starts_with(project_root) {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `folder` resolves outside the project root"
            )));
        }
        return Ok(canon);
    }

    // New nested folders: walk existing ancestors and reject escapes.
    let mut probe = target.to_path_buf();
    while !probe.exists() {
        if probe == project_root {
            break;
        }
        if let Some(parent) = probe.parent() {
            probe = parent.to_path_buf();
        } else {
            break;
        }
    }
    if probe.exists() {
        let canon = std::fs::canonicalize(&probe).map_err(|e| {
            AppError::Other(format!(
                "{TOOL_NAME}: canonicalize {:?}: {e}",
                probe
            ))
        })?;
        if !canon.starts_with(project_root) {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `folder` resolves outside the project root"
            )));
        }
    } else if !target.starts_with(project_root) {
        return Err(AppError::Invalid(format!(
            "{TOOL_NAME}: `folder` resolves outside the project root"
        )));
    }

    Ok(target.to_path_buf())
}

/// Sanitize one folder name for use as a single path segment.
fn sanitize_folder_segment(name: &str) -> String {
    sanitize_file_name(name)
}

/// Make a title safe to use as a file name across platforms. Strips path
/// separators and reserved characters; never returns an empty string.
fn sanitize_file_name(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let s = s.trim().trim_end_matches('.').trim().to_string();
    if s.is_empty() {
        "document".to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_breadcrumb_splits_slash_and_gt() {
        assert_eq!(
            parse_folder_breadcrumb("chapters/01").unwrap(),
            vec!["chapters".to_string(), "01".to_string()]
        );
        assert_eq!(
            parse_folder_breadcrumb("chapters > 01").unwrap(),
            vec!["chapters".to_string(), "01".to_string()]
        );
        assert_eq!(
            parse_folder_breadcrumb("notes").unwrap(),
            vec!["notes".to_string()]
        );
    }

    #[test]
    fn parse_breadcrumb_rejects_parent_dir() {
        assert!(parse_folder_breadcrumb("../escape").is_err());
        assert!(parse_folder_breadcrumb("a/../b").is_err());
    }

    #[test]
    fn resolve_output_dir_stays_under_project_root() {
        let root = std::env::temp_dir().join(format!(
            "moyan-createdoc-test-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let nested = resolve_output_dir(&root, Some("notes/drafts")).unwrap();
        assert!(nested.starts_with(
            std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone())
        ));

        assert!(resolve_output_dir(&root, Some("../outside")).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }
}
