//! Filesystem-scoped tool implementations.
//!
//! Today we ship one: [`FileReadTool`]. It mirrors the TS `FileReadTool`
//! in two important ways:
//!
//! - On a successful read, it records the absolute path in
//!   [`ToolUseContext::nested_memory_attachment_triggers`], so the
//!   runner's [`crate::ai::agent::memory::nested::collect_nested_memory`] pass
//!   can fire path-scoped `.claude/rules/*.md` injection on the next
//!   turn.
//! - On a successful read, it also records the path in
//!   [`ToolUseContext::read_file_state`] so subsequent reads of the same
//!   path can be de-duplicated by upstream callers.
//!
//! The tool handles common on-disk encodings (UTF-8/UTF-16/GBK) via
//! [`super::text_decode`]. Real callers usually prefer the host's native
//! file reader; this implementation exists primarily so the agent loop
//! has a working nested-memory trigger.

use std::path::PathBuf;

use serde_json::Value;

use crate::ai::agent::tools::paragraph::{number_paragraphs, paragraph_count};
use crate::ai::agent::tools::text_decode::decode_file_bytes;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "Read";

#[derive(Clone)]
pub struct FileReadTool {
    spec: ToolSpec,
}

impl Default for FileReadTool {
    fn default() -> Self {
        Self::new()
    }
}

impl FileReadTool {
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Read a text file from the local filesystem. \
                    Returns the full file content (never truncated) with each line \
                    prefixed by a label `[P001]`, `[P002]`, … (one line = one paragraph; \
                    empty lines are numbered too). Use these labels with Edit's \
                    `paragraph_number`. Supports UTF-8, UTF-16, and on Windows GBK/ANSI."
                    .to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the file to read."
                        }
                    },
                    "required": ["path"]
                }),
                read_only: true,
                concurrency_safe: true,
            },
        }
    }
}

impl Tool for FileReadTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid("Read: `path` must be a string".into()))?;
        if path.trim().is_empty() {
            return Err(AppError::Invalid("Read: `path` must be non-empty".into()));
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = invocation
                .input
                .get("path")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .ok_or_else(|| AppError::Invalid("Read: missing path".into()))?;

            // Relative paths would resolve against the host process CWD
            // (the app's own directory) — never allowed.
            if !path.is_absolute() {
                return Ok(ToolResult::error(format!(
                    "Read: `path` must be absolute, got `{}`",
                    path.display()
                )));
            }

            let canonical = std::fs::canonicalize(&path)
                .map_err(|e| AppError::Other(format!("Read: canonicalize {:?}: {e}", path)))?;

            let bytes = std::fs::read(&canonical)
                .map_err(|e| AppError::Other(format!("Read: open {:?}: {e}", canonical)))?;
            let text = decode_file_bytes(&bytes);
            let numbered = number_paragraphs(&text);
            let chars = text.chars().filter(|c| !c.is_whitespace()).count();
            let lines = text.lines().count();
            let paragraphs = paragraph_count(&text);

            // Record both for nested-memory injection and for the read
            // de-dup set on the active context.
            if let Ok(mut s) = invocation.context.nested_memory_attachment_triggers.lock() {
                s.insert(canonical.clone());
            }
            if let Ok(mut s) = invocation.context.read_file_state.lock() {
                s.insert(canonical.clone());
            }

            Ok(ToolResult::ok(serde_json::json!({
                "path": canonical.to_string_lossy(),
                "bytes": bytes.len(),
                "chars": chars,
                "lines": lines,
                "paragraphs": paragraphs,
                "text": numbered,
            })))
        })
    }
}
