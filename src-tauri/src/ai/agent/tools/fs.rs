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
use crate::ai::agent::tools::project_path::{self, display_path, FILE_REF_DESC};
use crate::ai::agent::tools::read_receipt::{
    content_hash, expand_read_range, MIN_READ_CONTEXT_LINES,
};
use crate::ai::agent::tools::text_decode::detect_and_decode;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "Read";

fn parse_p_number(s: &str) -> Option<usize> {
    let rest = s.strip_prefix('P').or_else(|| s.strip_prefix('p'))?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// Split `path` / `path#P003-P007` into `(clean_path, optional range)`.
///
/// Composer mentions serialize as `@"…"#P003-P007`; models sometimes paste that
/// suffix onto `path` instead of using `paragraph_from` / `paragraph_to`.
fn split_path_paragraph_suffix(raw: &str) -> AppResult<(String, Option<(usize, usize)>)> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Invalid("Read: `path` must be non-empty".into()));
    }
    let Some(hash) = trimmed.rfind('#') else {
        return Ok((trimmed.to_string(), None));
    };
    let suffix = &trimmed[hash + 1..];
    // `#P003` or `#P003-P007`
    let (from_raw, to_raw) = match suffix.split_once('-') {
        Some((a, b)) => (a, Some(b)),
        None => (suffix, None),
    };
    let Some(from) = parse_p_number(from_raw) else {
        // Not a paragraph suffix (e.g. URL fragment) — keep path as-is.
        return Ok((trimmed.to_string(), None));
    };
    if from < 1 {
        return Err(AppError::Invalid(
            "Read: `#P…` range on path must start at >= 1".into(),
        ));
    }
    let to = match to_raw {
        None => from,
        Some(raw) => {
            let Some(n) = parse_p_number(raw) else {
                return Ok((trimmed.to_string(), None));
            };
            n
        }
    };
    if to < from {
        return Err(AppError::Invalid(format!(
            "Read: path range `#P{from}-P{to}` has `to` < `from`"
        )));
    }
    let path = trimmed[..hash].trim_end();
    if path.is_empty() {
        return Err(AppError::Invalid(
            "Read: `path` must include a file before the `#P…` range suffix".into(),
        ));
    }
    Ok((path.to_string(), Some((from, to))))
}

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

/// Resolve the effective paragraph range: explicit tool args win; otherwise
/// fall back to a `#P003-P007` suffix on `path`.
fn resolve_read_target(
    raw_path: &str,
    paragraph_from: Option<usize>,
    paragraph_to: Option<usize>,
) -> AppResult<(String, Option<(usize, usize)>)> {
    let (path, suffix_range) = split_path_paragraph_suffix(raw_path)?;
    let explicit = resolve_paragraph_range(paragraph_from, paragraph_to)?;
    Ok((path, explicit.or(suffix_range)))
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
                    snippets into Edit's `old_string`. \
                    When the user message cites a ranged file mention like \
                    `@\"chapter.md\"#P003-P007` (or the chip label shows `· P003–P007`), \
                    call ranged Read for that span: set `path` to the file and pass \
                    `paragraph_from` / `paragraph_to` (1-based inclusive line numbers; \
                    one line = one paragraph). You may also append `#P003-P007` on `path` \
                    instead of the two args. Short ranges are auto-expanded with nearby \
                    context (at least 20 lines when the file is long enough). \
                    For open-ended prose tasks without a range mention, Read the full file \
                    once up front. After Edit fails, re-Read the relevant span before retrying. \
                    Do not re-read before every Edit."
                    .to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": format!(
                                "{FILE_REF_DESC} Optional `#P003` / `#P003-P007` suffix \
                                 selects a 1-based paragraph (line) range when \
                                 `paragraph_from` / `paragraph_to` are omitted."
                            )
                        },
                        "paragraph_from": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "First paragraph/line to return (1-based, inclusive). \
                                Prefer this over a `#P…` path suffix. Omit (with no suffix) to read the full file."
                        },
                        "paragraph_to": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Last paragraph/line to return (1-based, inclusive). \
                                Defaults to `paragraph_from` when omitted."
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
        let from = parse_optional_paragraph(input.get("paragraph_from"), "paragraph_from")?;
        let to = parse_optional_paragraph(input.get("paragraph_to"), "paragraph_to")?;
        resolve_read_target(path, from, to)?;
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let raw_path = invocation
                .input
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| AppError::Invalid("Read: missing path".into()))?;
            let from = parse_optional_paragraph(invocation.input.get("paragraph_from"), "paragraph_from")?;
            let to = parse_optional_paragraph(invocation.input.get("paragraph_to"), "paragraph_to")?;
            let (path, range) = resolve_read_target(raw_path, from, to)?;
            let canonical =
                project_path::resolve_project_file(&invocation.context.cwd, &path, TOOL_NAME)?;

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
            // Record the receipt against the *full* file content hash so
            // unchanged re-reads can be short-circuited, even when this Read
            // only returned a ranged window.
            if let Ok(mut s) = invocation.context.read_file_state.lock() {
                s.insert(canonical.clone(), content_hash(&text));
            }

            Ok(ToolResult::ok(serde_json::json!({
                "path": display_path(&canonical),
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

#[cfg(test)]
mod read_range_tests {
    use super::*;
    use crate::ai::agent::core::context::ToolUseContextBuilder;
    use crate::ai::agent::types::{AgentId, MessageId};
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    #[test]
    fn splits_path_paragraph_suffix() {
        let (path, range) = split_path_paragraph_suffix(r"drafts\chapter.md#P003-P007").unwrap();
        assert_eq!(path, r"drafts\chapter.md");
        assert_eq!(range, Some((3, 7)));

        let (path, range) = split_path_paragraph_suffix("notes.md#P12").unwrap();
        assert_eq!(path, "notes.md");
        assert_eq!(range, Some((12, 12)));

        let (path, range) = split_path_paragraph_suffix("notes.md").unwrap();
        assert_eq!(path, "notes.md");
        assert_eq!(range, None);
    }

    #[test]
    fn explicit_args_win_over_path_suffix() {
        let (path, range) =
            resolve_read_target("notes.md#P003-P007", Some(10), Some(12)).unwrap();
        assert_eq!(path, "notes.md");
        assert_eq!(range, Some((10, 12)));
    }

    #[tokio::test]
    async fn reads_range_from_path_suffix() {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "moyan-read-range-{}-{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let name = "chapter.txt";
        // 25 lines so a short request expands but still includes the span.
        let body: String = (1..=25).map(|i| format!("L{i}")).collect::<Vec<_>>().join("\n");
        std::fs::write(dir.join(name), &body).unwrap();
        let ctx = ToolUseContextBuilder::new(AgentId::new(), dir).build().0;
        let tool = FileReadTool::new();
        let res = tool
            .execute(ToolInvocation {
                id: MessageId("read".into()),
                input: json!({ "path": format!("{name}#P003-P005") }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(res.content["ranged"], true);
        assert_eq!(res.content["requested_paragraph_from"], 3);
        assert_eq!(res.content["requested_paragraph_to"], 5);
        let text = res.content["text"].as_str().unwrap();
        assert!(text.contains("L3"));
        assert!(text.contains("L5"));
        let _ = Arc::clone(&ctx);
    }
}
