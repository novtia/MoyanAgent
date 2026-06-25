//! File-mutation tools: `Write` (overwrite) and `Edit` (search/replace).
//!
//! Both refuse to operate on a file that the agent hasn't read first,
//! using [`crate::ai::agent::core::context::ToolUseContext::read_file_state`]
//! as the receipt. This mirrors the safety property in the TS executor
//! and prevents the model from clobbering files it doesn't actually
//! understand.
//!
//! The `Read` precondition is skipped for `Write` when the target file
//! does not yet exist — creating new files is always fine.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{FileOp, FileSnapshotStore};
use crate::ai::agent::tools::paragraph::{
    insert_paragraphs_after, join_paragraphs, replace_paragraph_with, split_paragraphs,
    strip_paragraph_label,
};
use crate::ai::agent::tools::text_decode::{
    detect_and_decode, normalize_tool_string, read_text_file, write_text_file, TextEncoding,
};
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const WRITE_TOOL: &str = "Write";
const EDIT_TOOL: &str = "Edit";

#[derive(Clone)]
pub struct FileWriteTool {
    spec: ToolSpec,
    snapshots: Arc<FileSnapshotStore>,
}

impl FileWriteTool {
    pub fn new(snapshots: Arc<FileSnapshotStore>) -> Self {
        Self {
            snapshots,
            spec: ToolSpec {
                name: WRITE_TOOL.to_string(),
                description: "Write a UTF-8 file to disk, creating parent directories as needed. \
                    Refuses to overwrite an existing file unless it was previously read in this session."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path":    { "type": "string", "description": "Absolute path to the file to write." },
                        "content": { "type": "string", "description": "Full file content. Overwrites existing data." }
                    },
                    "required": ["path", "content"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

impl Tool for FileWriteTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        require_nonempty_string(input, "path", WRITE_TOOL)?;
        require_string(input, "content", WRITE_TOOL)?;
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = path_arg(&invocation.input, WRITE_TOOL)?;
            let content = normalize_tool_string(
                invocation
                    .input
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );

            let exists = path.exists();
            if exists && !has_read_receipt(&invocation, &path) {
                return Ok(ToolResult::error(format!(
                    "Write: refusing to overwrite {} — read the file first to record a receipt",
                    path.display()
                )));
            }

            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        AppError::Other(format!("Write: mkdir {:?}: {e}", parent))
                    })?;
                }
            }

            // Snapshot the pre-image (before overwriting / creating) so the
            // change can be rolled back if its message is deleted.
            let op = if exists { FileOp::Update } else { FileOp::Create };
            self.snapshots
                .record_before(invocation.context.session_id.as_deref(), &path, op);

            let (encoding, had_bom) = if exists {
                let bytes = std::fs::read(&path).map_err(|e| {
                    AppError::Other(format!("Write: read {:?}: {e}", path))
                })?;
                let decoded = detect_and_decode(&bytes);
                (decoded.encoding, decoded.had_bom)
            } else {
                (TextEncoding::Utf8, false)
            };
            write_text_file(&path, &content, encoding, had_bom)
                .map_err(|e| AppError::Other(format!("Write: write {:?}: {e}", path)))?;

            record_read_receipt(&invocation, &path);

            let chars = content.chars().filter(|c| !c.is_whitespace()).count();
            let lines = if content.is_empty() {
                0
            } else {
                content.lines().count()
            };

            Ok(ToolResult::ok(json!({
                "path": path.to_string_lossy(),
                "bytes": content.len(),
                "created": !exists,
                "text": content,
                "chars": chars,
                "lines": lines,
            })))
        })
    }
}

#[derive(Clone)]
pub struct FileEditTool {
    spec: ToolSpec,
    snapshots: Arc<FileSnapshotStore>,
}

impl FileEditTool {
    pub fn new(snapshots: Arc<FileSnapshotStore>) -> Self {
        Self {
            snapshots,
            spec: ToolSpec {
                name: EDIT_TOOL.to_string(),
                description: "Edit a numbered paragraph in a file (Read labels `[P001]`, …; one line = one paragraph). \
                    Read the file once up front; then Edit from memory — do NOT Read before each Edit. \
                    Ranged Read only when Edit fails and you need the exact snippet. \
                    Bulk insert (append or mid-file): ONE Edit call — never one paragraph per call. \
                    Anchor on the paragraph at the insertion point (last paragraph to append; the \
                    paragraph immediately before the gap for mid-file insert). Set `original_content` \
                    to that paragraph verbatim and `modified_content` to the same text followed by \
                    ALL new lines (\\n-separated) in a single call. \
                    Small insert-after: set `paragraph_number` to N, leave `original_content` empty, \
                    put one or more new lines in `modified_content`. \
                    Fragment replace inside P00N: `original_content` = exact snippet, \
                    `modified_content` = replacement. \
                    Fill empty P00N: `original_content` empty and that paragraph is empty."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the file to edit."
                        },
                        "paragraph_number": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Target paragraph from Read ([P009] → 9). For bulk insert, the anchor paragraph (last para to append; para before the gap for mid-file)."
                        },
                        "original_content": {
                            "type": "string",
                            "description": "Bulk insert: anchor paragraph copied verbatim. Fragment replace: exact snippet inside the paragraph. Empty → small insert-after or fill empty paragraph."
                        },
                        "modified_content": {
                            "type": "string",
                            "description": "Bulk insert: anchor paragraph verbatim + \\n + ALL new lines in one call. Fragment replace: replacement text. Small insert-after: one or more new lines only."
                        }
                    },
                    "required": ["path", "paragraph_number", "original_content", "modified_content"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

impl Tool for FileEditTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        require_nonempty_string(input, "path", EDIT_TOOL)?;
        let para = input
            .get("paragraph_number")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                AppError::Invalid("Edit: `paragraph_number` must be a positive integer".into())
            })?;
        if para == 0 {
            return Err(AppError::Invalid(
                "Edit: `paragraph_number` must be >= 1".into(),
            ));
        }
        require_string(input, "original_content", EDIT_TOOL)?;
        require_string(input, "modified_content", EDIT_TOOL)?;
        if let (Some(o), Some(n)) = (
            input.get("original_content").and_then(Value::as_str),
            input.get("modified_content").and_then(Value::as_str),
        ) {
            if o == n {
                return Err(AppError::Invalid(
                    "Edit: `original_content` and `modified_content` are identical".into(),
                ));
            }
            if o.is_empty() && n.trim().is_empty() {
                return Err(AppError::Invalid(
                    "Edit: `modified_content` must be non-empty when inserting or filling".into(),
                ));
            }
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = path_arg(&invocation.input, EDIT_TOOL)?;
            let paragraph_number = invocation
                .input
                .get("paragraph_number")
                .and_then(Value::as_u64)
                .unwrap_or(0) as usize;
            let original = normalize_tool_string(strip_paragraph_label(
                invocation
                    .input
                    .get("original_content")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ));
            let modified = normalize_tool_string(strip_paragraph_label(
                invocation
                    .input
                    .get("modified_content")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ));

            if !has_read_receipt(&invocation, &path) {
                return Ok(ToolResult::error(format!(
                    "Edit: read {} first — no receipt on this session",
                    path.display()
                )));
            }

            let decoded = read_text_file(&path)
                .map_err(|e| AppError::Other(format!("Edit: read {:?}: {e}", path)))?;
            let mut paragraphs = split_paragraphs(&decoded.text);
            if paragraph_number == 0 || paragraph_number > paragraphs.len() {
                return Ok(ToolResult::error(format!(
                    "Edit: `paragraph_number` {paragraph_number} out of range (file has {} paragraphs)",
                    paragraphs.len()
                )));
            }
            let idx = paragraph_number - 1;
            let para = paragraphs[idx].as_str();
            let mut inserted = 0u32;
            let replaced = 1u32;

            if original.is_empty() {
                if para.trim().is_empty() {
                    paragraphs[idx] = modified.to_string();
                } else {
                    inserted = insert_paragraphs_after(&mut paragraphs, idx, &modified) as u32;
                    if inserted == 0 {
                        return Ok(ToolResult::error(
                            "Edit: `modified_content` must contain text to insert",
                        ));
                    }
                }
            } else if original == para || original == para.trim() {
                replace_paragraph_with(&mut paragraphs, idx, &modified);
            } else {
                let occurrences = para.matches(original.as_str()).count();
                if occurrences == 0 {
                    return Ok(ToolResult::error(format!(
                        "Edit: `original_content` not found in paragraph {paragraph_number} of {}",
                        path.display()
                    )));
                }
                if occurrences > 1 {
                    return Ok(ToolResult::error(format!(
                        "Edit: `original_content` appears {occurrences} times in paragraph {paragraph_number} — extend the fragment"
                    )));
                }
                paragraphs[idx] = para.replacen(original.as_str(), &modified, 1);
            }

            let updated = join_paragraphs(&paragraphs);

            // Snapshot the pre-image before mutating for rollback support.
            self.snapshots.record_before(
                invocation.context.session_id.as_deref(),
                &path,
                FileOp::Update,
            );

            write_text_file(&path, &updated, decoded.encoding, decoded.had_bom)
                .map_err(|e| AppError::Other(format!("Edit: write {:?}: {e}", path)))?;

            record_read_receipt(&invocation, &path);

            Ok(ToolResult::ok(json!({
                "path": path.to_string_lossy(),
                "paragraph_number": paragraph_number,
                "inserted": inserted,
                "replaced": if inserted > 0 { 0 } else { replaced },
                "paragraphs_total": paragraphs.len(),
            })))
        })
    }
}

fn require_nonempty_string(input: &Value, key: &str, tool: &str) -> AppResult<()> {
    let v = input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Invalid(format!("{tool}: `{key}` must be a string")))?;
    if v.is_empty() {
        return Err(AppError::Invalid(format!("{tool}: `{key}` must be non-empty")));
    }
    Ok(())
}

fn require_string(input: &Value, key: &str, tool: &str) -> AppResult<()> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Invalid(format!("{tool}: `{key}` must be a string")))?;
    Ok(())
}

fn path_arg(input: &Value, tool: &str) -> AppResult<PathBuf> {
    let raw = input
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Invalid(format!("{tool}: missing path")))?;
    let path = PathBuf::from(raw);
    // Relative paths would resolve against the host process CWD (the
    // app's own directory) — never allowed.
    if !path.is_absolute() {
        return Err(AppError::Invalid(format!(
            "{tool}: `path` must be absolute, got `{raw}`"
        )));
    }
    Ok(path)
}

/// True iff the agent has registered a read receipt for `path` (or its
/// canonical form) on the active context.
fn has_read_receipt(invocation: &ToolInvocation, path: &Path) -> bool {
    let canonical = std::fs::canonicalize(path).ok();
    let Ok(set) = invocation.context.read_file_state.lock() else {
        return false;
    };
    set.iter().any(|p| {
        if p == path {
            return true;
        }
        match (canonical.as_ref(), std::fs::canonicalize(p).ok()) {
            (Some(a), Some(b)) => a == &b,
            _ => false,
        }
    })
}

/// Stamp a read receipt after a successful write so subsequent edits in
/// the same session don't need an explicit `Read` round-trip.
fn record_read_receipt(invocation: &ToolInvocation, path: &Path) {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        if let Ok(mut s) = invocation.context.read_file_state.lock() {
            s.insert(canonical);
        }
    }
}
