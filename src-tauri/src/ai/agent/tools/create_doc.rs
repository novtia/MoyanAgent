//! `CreateDoc` — a deliberately tiny document-authoring tool.
//!
//! Unlike [`crate::ai::agent::tools::edit::FileWriteTool`] (which demands an
//! absolute path and a prior read receipt), `CreateDoc` asks the model for
//! only three things: a `title`, the `content`, and a `doc_type` (`md` or
//! `txt`). The file is written into the session's project working directory
//! (`ToolUseContext::cwd`), falling back to `Documents/MoYanAgent/Project` when the
//! session has no project path. The successful result echoes the full text
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
use crate::data::paths;

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
                    No path is required: the file is saved into the current project folder \
                    (or the user's Documents folder when no project is set). \
                    `doc_type` is `md` (Markdown) or `txt` (plain text). \
                    Prefer this over Write for authoring new documents — you only \
                    supply the title, the content, and the type."
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
            let title = normalize_tool_string(
                invocation
                    .input
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            )
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

            let ext = match doc_type.trim().to_lowercase().as_str() {
                "md" | "markdown" => "md",
                "txt" | "text" | "plain" => "txt",
                other => {
                    return Ok(ToolResult::error(format!(
                        "{TOOL_NAME}: unsupported doc_type `{other}` (use `md` or `txt`)"
                    )));
                }
            };

            let dir = resolve_output_dir(&invocation.context.cwd)?;
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
                "bytes": content.len(),
                "chars": chars,
                "lines": lines,
                "created": created,
                "text": content,
            })))
        })
    }
}

/// Resolve the directory to write into: the project working directory when
/// available and absolute, otherwise `Documents/MoYanAgent/Project`.
fn resolve_output_dir(cwd: &Path) -> AppResult<PathBuf> {
    if !cwd.as_os_str().is_empty() && cwd.is_absolute() {
        return Ok(cwd.to_path_buf());
    }
    paths::blank_projects_root()
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
