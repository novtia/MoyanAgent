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
//! The tool is intentionally minimal — no encoding detection, no image
//! handling, no `view_range`/`offset` controls. Real callers usually
//! prefer the host's native file reader; this implementation exists
//! primarily so the agent loop has a working nested-memory trigger.

use std::path::PathBuf;

use serde_json::Value;

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "Read";

/// Hard cap on bytes returned to the model. Larger files are truncated
/// and a `<truncated>` marker is appended.
const MAX_READ_BYTES: usize = 64 * 1024;

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
                description: "Read a UTF-8 text file from the local filesystem. \
                    Triggers nested-memory injection for the read path."
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
            // Decode the full file once for accurate word-count stats, then
            // cap the text actually returned to the model.
            let full_text = String::from_utf8_lossy(&bytes).into_owned();
            let (text, truncated) = cap_text(&full_text);

            // Word-count stats computed over the WHOLE file (not the capped
            // text): `chars` is the non-whitespace character count (counts CJK
            // characters; excludes spaces/tabs/newlines), `lines` the line count.
            let chars = full_text.chars().filter(|c| !c.is_whitespace()).count();
            let lines = full_text.lines().count();

            // Record both for nested-memory injection and for the read
            // de-dup set on the active context.
            if let Ok(mut s) = invocation.context.nested_memory_attachment_triggers.lock() {
                s.insert(canonical.clone());
            }
            if let Ok(mut s) = invocation.context.read_file_state.lock() {
                s.insert(canonical.clone());
            }

            let mut content = serde_json::json!({
                "path": canonical.to_string_lossy(),
                "bytes": bytes.len(),
                "chars": chars,
                "lines": lines,
                "text": text,
            });
            if truncated {
                content
                    .as_object_mut()
                    .unwrap()
                    .insert("truncated".into(), Value::Bool(true));
            }
            Ok(ToolResult::ok(content))
        })
    }
}

/// Cap the decoded text to `MAX_READ_BYTES` bytes for the model. Truncation
/// happens on a UTF-8 char boundary so we never split a multi-byte character.
fn cap_text(full: &str) -> (String, bool) {
    if full.len() <= MAX_READ_BYTES {
        return (full.to_string(), false);
    }
    // Walk back to the nearest char boundary at or below the cap.
    let mut end = MAX_READ_BYTES;
    while end > 0 && !full.is_char_boundary(end) {
        end -= 1;
    }
    let mut s = full[..end].to_string();
    s.push_str("\n\n<truncated>");
    (s, true)
}
