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

use serde_json::{json, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const WRITE_TOOL: &str = "Write";
const EDIT_TOOL: &str = "Edit";

#[derive(Clone)]
pub struct FileWriteTool {
    spec: ToolSpec,
}

impl Default for FileWriteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FileWriteTool {
    pub fn new() -> Self {
        Self {
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
            let content = invocation
                .input
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default();

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
            std::fs::write(&path, content.as_bytes())
                .map_err(|e| AppError::Other(format!("Write: write {:?}: {e}", path)))?;

            record_read_receipt(&invocation, &path);

            Ok(ToolResult::ok(json!({
                "path": path.to_string_lossy(),
                "bytes": content.len(),
                "created": !exists,
            })))
        })
    }
}

#[derive(Clone)]
pub struct FileEditTool {
    spec: ToolSpec,
}

impl Default for FileEditTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FileEditTool {
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                name: EDIT_TOOL.to_string(),
                description: "Replace `old_string` with `new_string` inside a file. \
                    Requires that the file was read in this session. \
                    Fails if `old_string` is not found or is ambiguous (unless `replace_all`)."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path":        { "type": "string" },
                        "old_string":  { "type": "string" },
                        "new_string":  { "type": "string" },
                        "replace_all": { "type": "boolean", "default": false }
                    },
                    "required": ["path", "old_string", "new_string"]
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
        require_nonempty_string(input, "old_string", EDIT_TOOL)?;
        require_string(input, "new_string", EDIT_TOOL)?;
        if let (Some(o), Some(n)) = (
            input.get("old_string").and_then(Value::as_str),
            input.get("new_string").and_then(Value::as_str),
        ) {
            if o == n {
                return Err(AppError::Invalid(
                    "Edit: `old_string` and `new_string` are identical".into(),
                ));
            }
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = path_arg(&invocation.input, EDIT_TOOL)?;
            let old = invocation
                .input
                .get("old_string")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let new = invocation
                .input
                .get("new_string")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let replace_all = invocation
                .input
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            if !has_read_receipt(&invocation, &path) {
                return Ok(ToolResult::error(format!(
                    "Edit: read {} first — no receipt on this session",
                    path.display()
                )));
            }

            let original = std::fs::read_to_string(&path)
                .map_err(|e| AppError::Other(format!("Edit: read {:?}: {e}", path)))?;
            let occurrences = original.matches(old).count();
            if occurrences == 0 {
                return Ok(ToolResult::error(format!(
                    "Edit: `old_string` not found in {}",
                    path.display()
                )));
            }
            if occurrences > 1 && !replace_all {
                return Ok(ToolResult::error(format!(
                    "Edit: `old_string` appears {occurrences} times in {} — pass `replace_all: true` or extend the snippet to be unique",
                    path.display()
                )));
            }

            let updated = if replace_all {
                original.replace(old, new)
            } else {
                original.replacen(old, new, 1)
            };
            std::fs::write(&path, updated.as_bytes())
                .map_err(|e| AppError::Other(format!("Edit: write {:?}: {e}", path)))?;

            record_read_receipt(&invocation, &path);

            Ok(ToolResult::ok(json!({
                "path": path.to_string_lossy(),
                "replaced": if replace_all { occurrences } else { 1 },
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
    Ok(PathBuf::from(raw))
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
