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

/// Largest slice of a document one `Read` may return, in characters.
///
/// A manuscript is routinely longer than the model's entire context window, so
/// an unranged read of one is a request the provider can only reject. The cap is
/// counted in characters rather than tokens because it has to hold for CJK prose
/// (roughly one token per glyph) as well as for ASCII, and the CJK case is the
/// expensive one.
const READ_MAX_CHARS: usize = 40_000;

/// Collect paragraphs `[from, to]`, stopping early once `max_chars` is spent.
///
/// Returns the slice, the last paragraph actually included, and whether the
/// requested range was cut short. The first paragraph is always taken even if it
/// alone busts the budget: returning nothing would read to the model as an empty
/// file rather than a truncated one. The caller trims that case separately.
fn collect_paragraphs(
    text: &str,
    from: usize,
    to: usize,
    max_chars: usize,
) -> (String, usize, bool) {
    let mut out = String::new();
    let mut chars = 0usize;
    let mut last = from.saturating_sub(1);
    let mut capped = false;

    for (i, line) in text.split('\n').enumerate() {
        let n = i + 1;
        if n < from {
            continue;
        }
        if n > to {
            break;
        }
        let line_chars = line.chars().count();
        if n > from && chars + line_chars + 1 > max_chars {
            capped = true;
            break;
        }
        if n > from {
            out.push('\n');
            chars += 1;
        }
        out.push_str(line);
        chars += line_chars;
        last = n;
    }

    (out, last, capped)
}

/// Trim a single over-long paragraph, keeping its head.
///
/// Reached when a document has no line breaks at all — one paragraph holding
/// the whole manuscript. Keeping the head (rather than eliding the middle) lets
/// the model continue reading forward with `paragraph_from`, which is the
/// affordance the result advertises.
fn head_limit(text: &str, max_chars: usize) -> Option<String> {
    if text.chars().count() <= max_chars {
        return None;
    }
    Some(text.chars().take(max_chars).collect())
}

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
                    once up front. A document title without `.md` / `.txt` is enough when it \
                    uniquely identifies the file. After Edit fails, re-Read the relevant span \
                    before retrying. \
                    Do not re-read before every Edit. \
                    Long files come back one page at a time: when the result has \
                    `truncated: true`, the text stops at `paragraph_to` and \
                    `next_paragraph_from` is where the following page starts. Continue from \
                    there only if you actually need the rest — prefer a targeted range over \
                    paging through a whole manuscript."
                    .to_string(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": format!(
                                "{FILE_REF_DESC} A `.md` / `.txt` suffix may be omitted \
                                 when the title uniquely identifies the document \
                                 (e.g. `notes` reads `notes.md`). \
                                 Optional `#P003` / `#P003-P007` suffix \
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
            crate::ai::agent::tools::pe_docs::refuse_nondoc(TOOL_NAME, &canonical)?;

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

            let (mut slice_text, mut paragraph_to, mut capped) =
                collect_paragraphs(&text, paragraph_from, paragraph_to, READ_MAX_CHARS);
            // A document with no line breaks is one enormous paragraph, which
            // the per-paragraph loop above cannot split.
            if let Some(head) = head_limit(&slice_text, READ_MAX_CHARS) {
                slice_text = head;
                capped = true;
            }
            if paragraph_to < paragraph_from {
                paragraph_to = paragraph_from;
            }
            let next_paragraph_from = if capped && paragraph_to < paragraphs_total {
                Some(paragraph_to + 1)
            } else {
                None
            };
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
                "truncated": capped,
                "next_paragraph_from": next_paragraph_from,
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
        assert_eq!(res.content["truncated"], false);
        assert!(res.content["next_paragraph_from"].is_null());
        let _ = Arc::clone(&ctx);
    }

    #[test]
    fn a_short_range_is_returned_whole() {
        let text = "one\ntwo\nthree\nfour";
        let (slice, last, capped) = collect_paragraphs(text, 2, 3, READ_MAX_CHARS);
        assert_eq!(slice, "two\nthree");
        assert_eq!(last, 3);
        assert!(!capped);
    }

    /// The regression: an unranged read of a manuscript used to return the whole
    /// thing, which on its own exceeded the context window.
    #[test]
    fn a_long_document_stops_at_the_page_boundary() {
        let body: String = (1..=5_000)
            .map(|i| format!("paragraph {i} {}", "w".repeat(200)))
            .collect::<Vec<_>>()
            .join("\n");
        let total = crate::ai::agent::tools::paragraph::paragraph_count(&body);
        let (slice, last, capped) = collect_paragraphs(&body, 1, total, READ_MAX_CHARS);
        assert!(capped, "a 1M-character document must not come back whole");
        assert!(slice.chars().count() <= READ_MAX_CHARS);
        assert!(last > 1, "at least one full page is returned");
        assert!(last < total, "there is more to page through");
        assert!(slice.starts_with("paragraph 1 "));
    }

    /// A novel saved without line breaks is one paragraph holding everything.
    #[test]
    fn a_single_enormous_paragraph_is_head_limited() {
        let body = "z".repeat(READ_MAX_CHARS * 3);
        let (slice, _, _) = collect_paragraphs(&body, 1, 1, READ_MAX_CHARS);
        assert!(slice.chars().count() > READ_MAX_CHARS, "loop cannot split it");
        let head = head_limit(&slice, READ_MAX_CHARS).expect("trimmed");
        assert_eq!(head.chars().count(), READ_MAX_CHARS);
    }

    #[tokio::test]
    async fn reads_a_document_by_title_without_the_suffix() {
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "moyan-read-extless-{}-{}",
            std::process::id(),
            n
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("深喉验毒.md"), "hello from the file").unwrap();
        let ctx = ToolUseContextBuilder::new(AgentId::new(), dir.clone())
            .build()
            .0;
        let tool = FileReadTool::new();
        let res = tool
            .execute(ToolInvocation {
                id: MessageId("read".into()),
                input: json!({ "path": "深喉验毒" }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(res.content["text"], "hello from the file");
        let path = res.content["path"].as_str().unwrap();
        assert!(
            path.ends_with("深喉验毒.md"),
            "result path should keep the real suffix, got {path}"
        );
        let _ = Arc::clone(&ctx);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
