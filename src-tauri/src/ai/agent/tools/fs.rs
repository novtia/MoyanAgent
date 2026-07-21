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

use serde_json::Value;

use crate::ai::agent::tools::paragraph::paragraph_count;
use crate::ai::agent::tools::project_path::{self, FILE_REF_DESC};
use crate::ai::agent::tools::read_receipt::{
    content_hash, expand_read_range, MIN_READ_CONTEXT_LINES,
};
use crate::ai::agent::tools::text_decode::detect_and_decode;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "Read";

fn parse_optional_paragraph(v: Option<&Value>, field: &str) -> AppResult<Option<usize>> {
    let Some(val) = v else {
        return Ok(None);
    };
    if val.is_null() {
        return Ok(None);
    }
    let n = val.as_i64().ok_or_else(|| {
        AppError::Invalid(format!("Read: `{field}` must be a positive integer"))
    })?;
    if n < 1 {
        return Err(AppError::Invalid(format!(
            "Read: `{field}` must be >= 1"
        )));
    }
    Ok(Some(n as usize))
}

fn resolve_paragraph_range(
    paragraph_from: Option<usize>,
    paragraph_to: Option<usize>,
) -> AppResult<Option<(usize, usize)>> {
    match (paragraph_from, paragraph_to) {
        (None, None) => Ok(None),
        (Some(from), None) => Ok(Some((from, from))),
        (Some(from), Some(to)) => {
            if to < from {
                return Err(AppError::Invalid(format!(
                    "Read: `paragraph_to` ({to}) must be >= `paragraph_from` ({from})"
                )));
            }
            Ok(Some((from, to)))
        }
        (None, Some(to)) => Err(AppError::Invalid(format!(
            "Read: `paragraph_from` is required when `paragraph_to` is {to}"
        ))),
    }
}

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
                    Returns the file's plain text (no line labels), so you can copy exact \
                    snippets into Edit's `old_string`. Read the target file once at the start \
                    of a prose task (full file is fine). Use ranged Read (`paragraph_from`, \
                    optional `paragraph_to`, treated as 1-based line numbers) ONLY when Edit \
                    failed and you need to re-read the exact text — you may request a single \
                    line; the system automatically expands the returned window to include \
                    surrounding context (at least 20 lines when the file is long enough). \
                    Do not re-read before every Edit."
                    .to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": FILE_REF_DESC
                        },
                        "paragraph_from": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "First line to return (1-based, inclusive). Omit to read the full file."
                        },
                        "paragraph_to": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Last line to return (1-based, inclusive). Defaults to `paragraph_from` when omitted."
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
        let from = parse_optional_paragraph(input.get("paragraph_from"), "paragraph_from")?;
        let to = parse_optional_paragraph(input.get("paragraph_to"), "paragraph_to")?;
        resolve_paragraph_range(from, to)?;
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = invocation
                .input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Invalid("Read: missing path".into()))?;
            let canonical = project_path::resolve_project_file(&invocation.context.cwd, path, TOOL_NAME)?;

            if !canonical.is_file() {
                return Ok(ToolResult::error(format!(
                    "Read: file not found: `{path}`"
                )));
            }

            let bytes = std::fs::read(&canonical)
                .map_err(|e| AppError::Other(format!("Read: open {:?}: {e}", canonical)))?;
            let decoded = detect_and_decode(&bytes);
            let text = decoded.text;
            let paragraphs_total = paragraph_count(&text);

            let from = parse_optional_paragraph(invocation.input.get("paragraph_from"), "paragraph_from")?;
            let to = parse_optional_paragraph(invocation.input.get("paragraph_to"), "paragraph_to")?;
            let range = resolve_paragraph_range(from, to)?;

            let (requested_from, requested_to, paragraph_from, paragraph_to, context_expanded) =
                match range {
                    None => (1, paragraphs_total, 1, paragraphs_total, false),
                    Some((f, t)) => {
                        if f == 0 || f > paragraphs_total {
                            return Ok(ToolResult::error(format!(
                                "Read: `paragraph_from` {f} out of range (file has {paragraphs_total} paragraphs)"
                            )));
                        }
                        if t > paragraphs_total {
                            return Ok(ToolResult::error(format!(
                                "Read: `paragraph_to` {t} out of range (file has {paragraphs_total} paragraphs)"
                            )));
                        }
                        let (expanded_from, expanded_to) =
                            expand_read_range(f, t, paragraphs_total);
                        let expanded = expanded_from != f || expanded_to != t;
                        (f, t, expanded_from, expanded_to, expanded)
                    }
                };

            let slice_text: String = text
                .split('\n')
                .enumerate()
                .filter_map(|(i, line)| {
                    let n = i + 1;
                    if n >= paragraph_from && n <= paragraph_to {
                        Some(line)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            let chars = slice_text.chars().filter(|c| !c.is_whitespace()).count();
            let paragraphs_returned = paragraph_to - paragraph_from + 1;
            let ranged = range.is_some();

            // Record both for nested-memory injection and for the read
            // de-dup set on the active context.
            if let Ok(mut s) = invocation.context.nested_memory_attachment_triggers.lock() {
                s.insert(canonical.clone());
            }
            // Record the receipt against the *full* file content hash so a
            // later Edit can tell whether the file changed out-of-band, even
            // when this Read only returned a ranged window.
            if let Ok(mut s) = invocation.context.read_file_state.lock() {
                s.insert(canonical.clone(), content_hash(&text));
            }

            Ok(ToolResult::ok(serde_json::json!({
                "path": canonical.to_string_lossy(),
                "bytes": bytes.len(),
                "encoding": decoded.encoding.label(),
                "had_bom": decoded.had_bom,
                "chars": chars,
                "lines": paragraphs_returned,
                "paragraphs_total": paragraphs_total,
                "paragraph_from": paragraph_from,
                "paragraph_to": paragraph_to,
                "requested_paragraph_from": requested_from,
                "requested_paragraph_to": requested_to,
                "context_expanded": context_expanded,
                "min_context_lines": MIN_READ_CONTEXT_LINES,
                "paragraphs_returned": paragraphs_returned,
                "ranged": ranged,
                "text": slice_text,
            })))
        })
    }
}
