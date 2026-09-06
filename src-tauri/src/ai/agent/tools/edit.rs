//! File-mutation tools: `Write` (overwrite) and `Edit` (string replace).
//!
//! `Edit` has one operation: find an `old_string` in the file and replace it
//! with `new_string`. By default `old_string` must match exactly once; if it
//! occurs multiple times the edit is rejected unless `replace_all` is set. An
//! empty `new_string` deletes the matched text.
//!
//! For insert/append the model is instructed to pass only a short unique
//! locator (the insertion point), not the whole chapter. The unmatched
//! remainder of the file is left untouched.
//!
//! Matching prefers a verbatim substring. If that misses, Edit retries with
//! unescaped JSON leftovers (`\"`), quote-folded text so ASCII `"`,
//! typographic `“”`, and CJK `「」` are treated as the same glyph, CRLF/LF
//! swaps, and — for a long locator — a unique head+tail span so a
//! paraphrased middle still lands on the intended passage. A folded
//! hit replaces the file's real span and rewrites `new_string` quotes to that
//! span's style, so a curly-quote model call does not change the file's
//! existing quote characters.
//! Successful Write/Edit still refresh the read receipt so unchanged-file
//! short-circuiting in Read stays accurate.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{FileChangeRecord, FileOp, FileSnapshotStore};
use crate::ai::agent::tools::project_path::{self, display_path, FILE_REF_DESC};
use crate::ai::agent::tools::read_receipt::{clear_receipt, record_receipt, DO_NOT_REREAD_NOTE};
use crate::ai::agent::tools::text_decode::{
    detect_and_decode, find_quote_folded_ranges, normalize_tool_string, read_text_file,
    remap_quotes_to_sample, write_text_file, TextEncoding,
};
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::data::db::DbPool;
use crate::data::pending_diff;
use crate::data::settings;
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
                    Overwrites the file if it already exists. After success, do NOT Read \
                    the file back — copy a short unique Edit `old_string` from the \
                    tail of `content` you just submitted, never the whole file."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path":    { "type": "string", "description": FILE_REF_DESC },
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
            // Strict resolution: a bare `notes.md` must land in the project
            // root, never on a same-named file the search happens to find in a
            // subfolder — Write overwrites, so guessing costs the user data.
            let path = strict_path_arg(&invocation.input, WRITE_TOOL, &invocation.context.cwd)?;
            crate::ai::agent::tools::pe_docs::refuse_nondoc(WRITE_TOOL, &path)?;
            // Written verbatim: JSON parsing already resolved the escapes, so any
            // remaining `\n` / `\\` / `\"` is literal source text.
            let content = invocation
                .input
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();

            // One read serves both the pre-image and the encoding to preserve,
            // so the snapshot describes exactly the bytes being replaced.
            let existing = match std::fs::read(&path) {
                Ok(bytes) => Some(detect_and_decode(&bytes)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => {
                    return Err(AppError::Other(format!("Write: read {:?}: {e}", path)));
                }
            };
            let exists = existing.is_some();
            let (encoding, had_bom) = existing
                .as_ref()
                .map(|d| (d.encoding, d.had_bom))
                .unwrap_or((TextEncoding::Utf8, false));

            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| AppError::Other(format!("Write: mkdir {:?}: {e}", parent)))?;
                }
            }

            // Snapshot the pre-image (before overwriting / creating) so the
            // change can be rolled back if its message is deleted. A failure
            // here aborts the write: an unrecorded mutation is unrecoverable.
            let change = match existing.as_ref() {
                Some(decoded) => FileChangeRecord::from_read(
                    &path,
                    FileOp::Update,
                    &decoded.text,
                    decoded.encoding,
                    decoded.had_bom,
                ),
                None => FileChangeRecord::absent(&path, FileOp::Create),
            };
            self.snapshots.record(
                invocation.context.session_id.as_deref(),
                invocation.context.correlation_id.as_deref(),
                &change,
            )?;

            write_text_file(&path, &content, encoding, had_bom)
                .map_err(|e| AppError::Other(format!("Write: write {:?}: {e}", path)))?;

            record_receipt(&invocation.context.read_file_state, &path, &content, true);

            let chars = content.chars().filter(|c| !c.is_whitespace()).count();
            let lines = if content.is_empty() {
                0
            } else {
                content.lines().count()
            };

            Ok(ToolResult::ok(json!({
                "path": display_path(&path),
                "bytes": content.len(),
                "created": !exists,
                "chars": chars,
                "lines": lines,
                "note": DO_NOT_REREAD_NOTE,
            })))
        })
    }
}

#[derive(Clone)]
pub struct FileEditTool {
    spec: ToolSpec,
    snapshots: Arc<FileSnapshotStore>,
    /// When set, successful Edits are recorded for the reader Keep/Undo UI.
    pool: Option<Arc<DbPool>>,
}

impl FileEditTool {
    pub fn new(snapshots: Arc<FileSnapshotStore>) -> Self {
        Self {
            snapshots,
            pool: None,
            spec: ToolSpec {
                name: EDIT_TOOL.to_string(),
                description: "Surgically replace one unique span in a file. This is not a \
                    full-file rewrite. \
                    Insert / append: `old_string` is ONLY a short unique locator \
                    at the insertion point (the sentence or paragraph ending you insert \
                    after). NEVER paste the whole chapter, the rest of the file, or all \
                    existing prose into `old_string`. `new_string` = that same locator \
                    followed by the new prose (or the locator with the insert spliced in). \
                    Unselected text before and after the locator is kept as-is — do not \
                    retype it. \
                    Rewrite or expand a passage: first Read the target paragraphs, then \
                    copy `old_string` verbatim from that Read output — do not reconstruct \
                    it from memory. `old_string` is that passage only. Delete: empty \
                    `new_string`. Full-document rewrite is the exception — only then may \
                    `old_string` be the entire file. \
                    `old_string` must match once unless `replace_all` is true. \
                    ASCII `\"` and typographic `“”`/`「」` match equivalently."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": format!(
                                "{FILE_REF_DESC} A `.md` / `.txt` suffix may be omitted \
                                 when the title uniquely identifies the document."
                            )
                        },
                        "old_string": {
                            "type": "string",
                            "description": "Locator copied from the file. \
                                For insert/append, copy only the sentence or \
                                paragraph ending at the insertion point — never the \
                                whole chapter or the rest of the document. \
                                For rewrite/expand, copy the target passage verbatim \
                                from a Read of those paragraphs. \
                                Must match once unless `replace_all` is true."
                        },
                        "new_string": {
                            "type": "string",
                            "description": "Replacement for that locator only; the rest \
                                of the file is left untouched. For insert/append, start \
                                with the same locator then continue with the new prose. \
                                Document body only — no explanations, notes, or change \
                                logs. Fill this in LAST, after path/old_string."
                        },
                        "replace_all": {
                            "type": "boolean",
                            "description": "Replace every occurrence of `old_string` instead of requiring a unique match. Defaults to false."
                        }
                    },
                    "required": ["path", "old_string", "new_string"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }

    pub fn with_pool(mut self, pool: Arc<DbPool>) -> Self {
        self.pool = Some(pool);
        self
    }

    fn replace_all_default(&self) -> bool {
        let Some(pool) = &self.pool else {
            return false;
        };
        let Ok(conn) = pool.get() else {
            return false;
        };
        settings::read_edit_replace_all_default(&conn)
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
        require_optional_bool(input, "replace_all", EDIT_TOOL)?;
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = path_arg(&invocation.input, EDIT_TOOL, &invocation.context.cwd)?;
            crate::ai::agent::tools::pe_docs::refuse_nondoc(EDIT_TOOL, &path)?;
            // Verbatim strings: JSON parsing already resolved the escapes, so any
            // remaining `\n` / `\\` / `\"` is literal source text to match as-is.
            let raw_old = invocation
                .input
                .get("old_string")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let raw_new = invocation
                .input
                .get("new_string")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let replace_all = invocation
                .input
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| self.replace_all_default());

            if raw_old.is_empty() {
                return Ok(ToolResult::error(
                    "Edit: `old_string` must be non-empty".to_string(),
                ));
            }
            if raw_old == raw_new {
                return Ok(ToolResult::error(identical_error()));
            }

            let decoded = read_text_file(&path)
                .map_err(|e| AppError::Other(format!("Edit: read {:?}: {e}", path)))?;

            let plan = match plan_edit(&decoded.text, &raw_old, &raw_new) {
                Ok(plan) => plan,
                Err(MatchError::NotFound) => {
                    clear_receipt(&invocation.context.read_file_state, &path);
                    return Ok(not_found_error(&path));
                }
                Err(MatchError::Identical) => {
                    return Ok(ToolResult::error(identical_error()));
                }
            };
            let occurrences = plan.occurrences();
            if occurrences > 1 && !replace_all {
                clear_receipt(&invocation.context.read_file_state, &path);
                return Ok(not_unique_error(occurrences));
            }

            let text_before = decoded.text.clone();
            let applied = apply_edit(&decoded.text, plan, replace_all);
            if applied.updated == text_before {
                return Ok(ToolResult::error(identical_error()));
            }
            let AppliedEdit {
                updated,
                old_string,
                new_string,
                replaced_count,
                match_start,
            } = applied;

            // Snapshot the pre-image before mutating for rollback support,
            // reusing the read the replacement was computed from: re-reading
            // here could capture a different file than the one being edited.
            self.snapshots.record(
                invocation.context.session_id.as_deref(),
                invocation.context.correlation_id.as_deref(),
                &FileChangeRecord::from_read(
                    &path,
                    FileOp::Update,
                    &text_before,
                    decoded.encoding,
                    decoded.had_bom,
                ),
            )?;

            write_text_file(&path, &updated, decoded.encoding, decoded.had_bom)
                .map_err(|e| AppError::Other(format!("Edit: write {:?}: {e}", path)))?;

            record_receipt(&invocation.context.read_file_state, &path, &updated, true);

            // Same key the snapshot row uses, so review rows and rollbacks
            // resolve to one identity per file.
            let path_str = crate::data::paths::normalized_path_key(&path);
            let mut pending_diff_id: Option<String> = None;
            if let (Some(pool), Some(sid)) = (&self.pool, invocation.context.session_id.as_deref())
            {
                if let Ok(conn) = pool.get() {
                    match pending_diff::insert(
                        &conn,
                        sid,
                        &path_str,
                        &old_string,
                        &new_string,
                        &text_before,
                        &updated,
                        Some(decoded.encoding.label()),
                        decoded.had_bom,
                        invocation.context.correlation_id.as_deref(),
                    ) {
                        Ok(Some(id)) => pending_diff_id = Some(id),
                        Ok(None) => {}
                        Err(e) => {
                            eprintln!("Edit: failed to record pending diff: {e}");
                        }
                    }
                }
            }

            let chars = updated.chars().filter(|c| !c.is_whitespace()).count();
            let mut body = json!({
                "success": true,
                "path": path_str,
                "old_string": old_string,
                "new_string": new_string,
                "replace_all": replace_all,
                "replaced_count": replaced_count,
                "match_start": match_start,
                "chars": chars,
            });
            if let Some(id) = pending_diff_id {
                body["pending_diff_id"] = json!(id);
            }

            Ok(ToolResult::ok(body))
        })
    }
}

fn not_found_error(path: &Path) -> ToolResult {
    ToolResult::error(format!(
        "Edit: `old_string` not found in {}. Read the file again and copy the exact text \
         (quotes in JSON show as \\\" — that is ASCII `\"`, not `“`/`”`).",
        path.display()
    ))
}

fn not_unique_error(occurrences: usize) -> ToolResult {
    ToolResult::error(format!(
        "Edit: `old_string` matched {occurrences} places — add more surrounding context to make it unique, or set `replace_all` to true."
    ))
}

fn identical_error() -> String {
    "Edit: `old_string` and `new_string` are identical — nothing to change".to_string()
}

enum MatchError {
    NotFound,
    Identical,
}

enum MatchPlan {
    Exact {
        old: String,
        new: String,
        occurrences: usize,
    },
    Folded {
        ranges: Vec<(usize, usize)>,
        new: String,
    },
    /// One byte range in the file (unique head+tail of a long paraphrased locator).
    Span {
        start: usize,
        end: usize,
        new: String,
    },
}

impl MatchPlan {
    fn occurrences(&self) -> usize {
        match self {
            Self::Exact { occurrences, .. } => *occurrences,
            Self::Folded { ranges, .. } => ranges.len(),
            Self::Span { .. } => 1,
        }
    }
}

struct AppliedEdit {
    updated: String,
    old_string: String,
    new_string: String,
    replaced_count: usize,
    match_start: usize,
}

/// Prefer a verbatim substring; then unescaped JSON leftovers; then quote-folded
/// matching so ASCII / curly / CJK quotes locate the same span; then CRLF/LF
/// swaps; then a unique head+tail span for a long paraphrased locator.
fn plan_edit(file: &str, raw_old: &str, raw_new: &str) -> Result<MatchPlan, MatchError> {
    if let Some(plan) = plan_literal(file, raw_old, raw_new) {
        return Ok(plan);
    }

    let unescaped_old = normalize_tool_string(raw_old);
    if unescaped_old != raw_old {
        let unescaped_new = normalize_tool_string(raw_new);
        if let Some(plan) = plan_literal(file, &unescaped_old, &unescaped_new) {
            if unescaped_old == unescaped_new {
                return Err(MatchError::Identical);
            }
            return Ok(plan);
        }
        if let Some(plan) = plan_newline(file, &unescaped_old, &unescaped_new) {
            return Ok(plan);
        }
        if let Some(plan) = plan_anchor_span(file, &unescaped_old, &unescaped_new) {
            return Ok(plan);
        }
    }

    if let Some(plan) = plan_newline(file, raw_old, raw_new) {
        return Ok(plan);
    }
    if let Some(plan) = plan_anchor_span(file, raw_old, raw_new) {
        return Ok(plan);
    }

    Err(MatchError::NotFound)
}

fn plan_literal(file: &str, old: &str, new: &str) -> Option<MatchPlan> {
    let exact = file.matches(old).count();
    if exact > 0 {
        return Some(MatchPlan::Exact {
            old: old.to_string(),
            new: new.to_string(),
            occurrences: exact,
        });
    }
    let folded = find_quote_folded_ranges(file, old);
    if !folded.is_empty() {
        return Some(MatchPlan::Folded {
            ranges: folded,
            new: new.to_string(),
        });
    }
    None
}

fn plan_newline(file: &str, old: &str, new: &str) -> Option<MatchPlan> {
    let swapped = newline_swapped(old)?;
    let plan = plan_literal(file, &swapped, &align_newlines(new, old, &swapped))?;
    Some(plan)
}

fn newline_swapped(s: &str) -> Option<String> {
    if s.contains("\r\n") {
        Some(s.replace("\r\n", "\n"))
    } else if s.contains('\n') {
        Some(s.replace('\n', "\r\n"))
    } else {
        None
    }
}

fn align_newlines(new: &str, raw_old: &str, matched_old: &str) -> String {
    let raw_crlf = raw_old.contains("\r\n");
    let matched_crlf = matched_old.contains("\r\n");
    if matched_crlf && !raw_crlf {
        new.replace('\n', "\r\n")
    } else if !matched_crlf && raw_crlf {
        new.replace("\r\n", "\n")
    } else {
        new.to_string()
    }
}

const MIN_ANCHOR_OLD_CHARS: usize = 80;
const MIN_ANCHOR_CHARS: usize = 24;
const MAX_ANCHOR_CHARS: usize = 80;

fn plan_anchor_span(file: &str, old: &str, new: &str) -> Option<MatchPlan> {
    let (start, end) = find_anchor_span(file, old)?;
    Some(MatchPlan::Span {
        start,
        end,
        new: new.to_string(),
    })
}

fn find_anchor_span(file: &str, old: &str) -> Option<(usize, usize)> {
    let old_chars = old.chars().count();
    if old_chars < MIN_ANCHOR_OLD_CHARS {
        return None;
    }
    let head = unique_growing_prefix(file, old)?;
    let tail = unique_growing_suffix(file, old)?;
    let start = unique_byte_start(file, head)?;
    let tail_at = unique_byte_start(file, tail)?;
    let end = tail_at + tail.len();
    if start >= end {
        return None;
    }
    let span_chars = file[start..end].chars().count();
    // Reject a span that is wildly shorter or longer than the locator.
    if span_chars * 5 < old_chars * 2 || span_chars * 2 > old_chars * 5 {
        return None;
    }
    Some((start, end))
}

fn unique_growing_prefix<'a>(file: &str, old: &'a str) -> Option<&'a str> {
    let max = old.chars().count() / 2;
    let mut n = MIN_ANCHOR_CHARS;
    while n <= max && n <= MAX_ANCHOR_CHARS {
        let head = prefix_chars(old, n);
        if unique_byte_start(file, head).is_some() {
            return Some(head);
        }
        n += 8;
    }
    None
}

fn unique_growing_suffix<'a>(file: &str, old: &'a str) -> Option<&'a str> {
    let max = old.chars().count() / 2;
    let mut n = MIN_ANCHOR_CHARS;
    while n <= max && n <= MAX_ANCHOR_CHARS {
        let tail = suffix_chars(old, n);
        if unique_byte_start(file, tail).is_some() {
            return Some(tail);
        }
        n += 8;
    }
    None
}

fn prefix_chars(s: &str, n: usize) -> &str {
    match s.char_indices().nth(n) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

fn suffix_chars(s: &str, n: usize) -> &str {
    let total = s.chars().count();
    if total <= n {
        return s;
    }
    match s.char_indices().nth(total - n) {
        Some((i, _)) => &s[i..],
        None => s,
    }
}

fn unique_byte_start(hay: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    let mut it = hay.match_indices(needle);
    let first = it.next()?;
    if it.next().is_some() {
        return None;
    }
    Some(first.0)
}

fn apply_edit(file: &str, plan: MatchPlan, replace_all: bool) -> AppliedEdit {
    match plan {
        MatchPlan::Exact {
            old,
            new,
            occurrences,
        } => {
            let match_start = file
                .find(&old)
                .map(|byte_idx| file[..byte_idx].chars().count())
                .unwrap_or(0);
            let (updated, replaced_count) = if replace_all {
                (file.replace(&old, &new), occurrences)
            } else {
                (file.replacen(&old, &new, 1), 1)
            };
            AppliedEdit {
                updated,
                old_string: old,
                new_string: new,
                replaced_count,
                match_start,
            }
        }
        MatchPlan::Folded { ranges, new } => {
            let use_ranges: Vec<(usize, usize)> = if replace_all {
                ranges
            } else {
                ranges.into_iter().take(1).collect()
            };
            let (first_start, first_end) = use_ranges[0];
            let match_start = file[..first_start].chars().count();
            let first_old = file[first_start..first_end].to_string();
            let first_new = remap_quotes_to_sample(&new, &first_old);
            let mut out = String::with_capacity(file.len().saturating_add(new.len()));
            let mut last = 0;
            for &(start, end) in &use_ranges {
                out.push_str(&file[last..start]);
                let span = &file[start..end];
                out.push_str(&remap_quotes_to_sample(&new, span));
                last = end;
            }
            out.push_str(&file[last..]);
            AppliedEdit {
                updated: out,
                old_string: first_old,
                new_string: first_new,
                replaced_count: use_ranges.len(),
                match_start,
            }
        }
        MatchPlan::Span { start, end, new } => {
            let match_start = file[..start].chars().count();
            let old = file[start..end].to_string();
            let mut updated = String::with_capacity(file.len().saturating_add(new.len()));
            updated.push_str(&file[..start]);
            updated.push_str(&new);
            updated.push_str(&file[end..]);
            AppliedEdit {
                updated,
                old_string: old,
                new_string: new,
                replaced_count: 1,
                match_start,
            }
        }
    }
}

fn require_nonempty_string(input: &Value, key: &str, tool: &str) -> AppResult<()> {
    let v = input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Invalid(format!("{tool}: `{key}` must be a string")))?;
    if v.is_empty() {
        return Err(AppError::Invalid(format!(
            "{tool}: `{key}` must be non-empty"
        )));
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

fn require_optional_bool(input: &Value, key: &str, tool: &str) -> AppResult<()> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(()),
        Some(v) if v.is_boolean() => Ok(()),
        Some(_) => Err(AppError::Invalid(format!(
            "{tool}: `{key}` must be a boolean"
        ))),
    }
}

fn path_arg(input: &Value, tool: &str, cwd: &Path) -> AppResult<PathBuf> {
    let raw = raw_path_arg(input, tool)?;
    project_path::resolve_project_file(cwd, raw, tool)
}

fn strict_path_arg(input: &Value, tool: &str, cwd: &Path) -> AppResult<PathBuf> {
    let raw = raw_path_arg(input, tool)?;
    project_path::resolve_project_file_strict(cwd, raw, tool)
}

fn raw_path_arg<'a>(input: &'a Value, tool: &str) -> AppResult<&'a str> {
    input
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Invalid(format!("{tool}: missing path")))
}

#[cfg(test)]
mod edit_tests {
    use super::*;
    use crate::ai::agent::core::context::{ToolUseContext, ToolUseContextBuilder};
    use crate::ai::agent::tools::fs::FileReadTool;
    use crate::ai::agent::types::{AgentId, MessageId};
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn test_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("moyan-edit-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Seed a file with `text`, returning `(context rooted at the dir, file name)`.
    fn seed(text: &str) -> (Arc<ToolUseContext>, String) {
        let dir = test_dir();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let name = format!("chapter-{n}.txt");
        std::fs::write(dir.join(&name), text).unwrap();
        let ctx = ToolUseContextBuilder::new(AgentId::new(), dir).build().0;
        (ctx, name)
    }

    /// Fresh context plus an unused file name; nothing is written to disk.
    fn blank() -> (Arc<ToolUseContext>, String) {
        let dir = test_dir();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let name = format!("script-{n}.mjs");
        let ctx = ToolUseContextBuilder::new(AgentId::new(), dir).build().0;
        (ctx, name)
    }

    async fn read_receipt(ctx: &Arc<ToolUseContext>, name: &str) {
        let tool = FileReadTool::new();
        tool.execute(ToolInvocation {
            id: MessageId("read".into()),
            input: json!({ "path": name }),
            context: ctx.as_ref(),
        })
        .await
        .unwrap();
    }

    async fn run_write(ctx: &Arc<ToolUseContext>, input: Value) -> ToolResult {
        let tool = FileWriteTool::new(Arc::new(FileSnapshotStore::new()));
        tool.validate(&input).unwrap();
        tool.execute(ToolInvocation {
            id: MessageId("write".into()),
            input,
            context: ctx.as_ref(),
        })
        .await
        .unwrap()
    }

    async fn run_edit(ctx: &Arc<ToolUseContext>, input: Value) -> ToolResult {
        if input.get("path").is_none() {
            panic!("edit test input needs a path");
        }
        let tool = FileEditTool::new(Arc::new(FileSnapshotStore::new()));
        tool.validate(&input).unwrap();
        tool.execute(ToolInvocation {
            id: MessageId("edit".into()),
            input,
            context: ctx.as_ref(),
        })
        .await
        .unwrap()
    }

    fn disk(ctx: &Arc<ToolUseContext>, name: &str) -> String {
        let path = ctx.cwd.join(name);
        let bytes = std::fs::read(&path).unwrap();
        detect_and_decode(&bytes).text
    }

    #[tokio::test]
    async fn replaces_unique_substring() {
        let (ctx, name) = seed("A\nB\nC\nD");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "B\nC", "new_string": "X\nY\nZ" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nX\nY\nZ\nD");
        assert_eq!(res.content["success"], true);
        assert_eq!(res.content["old_string"], "B\nC");
        assert_eq!(res.content["new_string"], "X\nY\nZ");
        assert_eq!(res.content["replace_all"], false);
        assert_eq!(res.content["replaced_count"], 1);
        assert_eq!(res.content["match_start"], 2);
        assert!(res.content.get("text").is_none());
        assert!(res.content.get("text_before").is_none());
    }

    #[tokio::test]
    async fn replaces_single_line() {
        let (ctx, name) = seed("A\nB\nC\nD");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "B", "new_string": "X" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nX\nC\nD");
    }

    #[tokio::test]
    async fn continue_by_replacing_tail_with_its_text_plus_new() {
        let (ctx, name) = seed("A\nB\n哦哦哦");
        read_receipt(&ctx, &name).await;
        // Continuation is expressed as replacing the tail with its existing
        // text followed by the new prose.
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "哦哦哦", "new_string": "哦哦哦。后续新内容\n再一段" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nB\n哦哦哦。后续新内容\n再一段");
        assert_eq!(res.content["old_string"], "哦哦哦");
    }

    #[tokio::test]
    async fn empty_new_string_deletes_match() {
        let (ctx, name) = seed("A\nB\nC\nD");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "\nB\nC", "new_string": "" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nD");
        assert_eq!(res.content["replaced_count"], 1);
    }

    #[tokio::test]
    async fn rejects_non_unique_match_without_replace_all() {
        let (ctx, name) = seed("X\nB\nX\nD");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "X", "new_string": "Q" }),
        )
        .await;
        assert!(res.is_error);
        // Nothing written.
        assert_eq!(disk(&ctx, &name), "X\nB\nX\nD");
    }

    #[tokio::test]
    async fn replace_all_replaces_every_occurrence() {
        let (ctx, name) = seed("X\nB\nX\nD");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "X", "new_string": "Q", "replace_all": true }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "Q\nB\nQ\nD");
        assert_eq!(res.content["replaced_count"], 2);
    }

    #[tokio::test]
    async fn omitted_replace_all_follows_settings_default() {
        let db = crate::data::db::test_support::TempDb::new("edit-replace-all");
        crate::data::settings::apply_patch(
            &db.conn(),
            crate::data::settings::SettingsPatch {
                edit_replace_all_default: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        let (ctx, name) = seed("X\nB\nX\nD");
        let tool = FileEditTool::new(Arc::new(FileSnapshotStore::new()))
            .with_pool(Arc::new(db.pool()));
        let res = tool
            .execute(ToolInvocation {
                id: MessageId("edit".into()),
                input: json!({ "path": name, "old_string": "X", "new_string": "Q" }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "Q\nB\nQ\nD");
        assert_eq!(res.content["replaced_count"], 2);
        assert_eq!(res.content["replace_all"], true);
    }

    #[tokio::test]
    async fn explicit_replace_all_false_overrides_settings_default() {
        let db = crate::data::db::test_support::TempDb::new("edit-replace-all-off");
        crate::data::settings::apply_patch(
            &db.conn(),
            crate::data::settings::SettingsPatch {
                edit_replace_all_default: Some(true),
                ..Default::default()
            },
        )
        .unwrap();
        let (ctx, name) = seed("X\nB\nX\nD");
        let tool = FileEditTool::new(Arc::new(FileSnapshotStore::new()))
            .with_pool(Arc::new(db.pool()));
        let res = tool
            .execute(ToolInvocation {
                id: MessageId("edit".into()),
                input: json!({
                    "path": name,
                    "old_string": "X",
                    "new_string": "Q",
                    "replace_all": false
                }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(res.is_error);
        assert_eq!(disk(&ctx, &name), "X\nB\nX\nD");
    }

    #[tokio::test]
    async fn rejects_old_string_not_found() {
        let (ctx, name) = seed("A\nB");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "ZZZ", "new_string": "X" }),
        )
        .await;
        assert!(res.is_error);
        assert_eq!(disk(&ctx, &name), "A\nB");
    }

    #[tokio::test]
    async fn rejects_identical_old_and_new() {
        let (ctx, name) = seed("A\nB");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "A", "new_string": "A" }),
        )
        .await;
        assert!(res.is_error);
        assert_eq!(disk(&ctx, &name), "A\nB");
    }

    #[tokio::test]
    async fn edit_applies_even_if_disk_changed_out_of_band() {
        let (ctx, name) = seed("A\nB\nC");
        read_receipt(&ctx, &name).await;
        // Out-of-band change must not block Edit when old_string still matches.
        std::fs::write(ctx.cwd.join(&name), "A\nB\nC\nD\nE").unwrap();
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "B", "new_string": "Z" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nZ\nC\nD\nE");
    }

    #[tokio::test]
    async fn edit_without_prior_read_succeeds() {
        let (ctx, name) = seed("A\nB");
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "A", "new_string": "Z" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "Z\nB");
    }

    #[tokio::test]
    async fn write_preserves_source_escapes_verbatim() {
        let (ctx, name) = blank();
        let content = r#"fail('syntax: ' + relative(root, f) + ' -> ' + err.split('\n')[0]);
const dir = "C:\\Users\\Administrator";
const quoted = "say \"hi\"";
const entity = "a &amp; b";
const re = /\d+\\s/g;
"#;
        let res = run_write(&ctx, json!({ "path": &name, "content": content })).await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), content);
        assert!(res.content.get("text").is_none());
        assert_eq!(res.content["created"], true);
    }

    #[tokio::test]
    async fn write_does_not_collapse_double_backslashes() {
        let (ctx, name) = blank();
        let content = r#"\\\\ \\ \ \n \t"#;
        let res = run_write(&ctx, json!({ "path": &name, "content": content })).await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), content);
    }

    #[tokio::test]
    async fn edit_matches_backslashes_verbatim() {
        let (ctx, name) = seed(r#"const parts = err.split('\n');"#);
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({
                "path": name,
                "old_string": r#"err.split('\n')"#,
                "new_string": r#"err.split('\r\n')"#,
            }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), r#"const parts = err.split('\r\n');"#);
    }

    #[tokio::test]
    async fn edit_falls_back_to_unescaped_prose_when_raw_misses() {
        let (ctx, name) = seed("他说\"你好\"。");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({
                "path": name,
                "old_string": r#"他说\"你好\"。"#,
                "new_string": r#"他说\"再见\"。"#,
            }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "他说\"再见\"。");
    }

    #[tokio::test]
    async fn edit_prefers_raw_match_over_unescaped_fallback() {
        // Both readings exist in the file; the verbatim one must win.
        let (ctx, name) = seed("literal: a\\nb\nprose: a\nb\n");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "a\\nb", "new_string": "OK" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "literal: OK\nprose: a\nb\n");
    }

    /// Write overwrites, so a bare file name must not be retargeted at a
    /// same-named file living in a subfolder.
    #[tokio::test]
    async fn write_creates_at_the_root_instead_of_overwriting_a_nested_namesake() {
        let dir = test_dir().join(format!("strict-{}", COUNTER.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(dir.join("chapters")).unwrap();
        let nested = dir.join("chapters").join("outline.md");
        std::fs::write(&nested, "precious").unwrap();
        let ctx = ToolUseContextBuilder::new(AgentId::new(), dir.clone())
            .build()
            .0;

        let res = run_write(&ctx, json!({ "path": "outline.md", "content": "new" })).await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(
            std::fs::read_to_string(&nested).unwrap(),
            "precious",
            "the nested namesake must be untouched"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("outline.md")).unwrap(),
            "new"
        );
        assert_eq!(res.content["created"], true);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn consecutive_edits_work_without_reread() {
        let (ctx, name) = seed("A\nB\nC");
        let r1 = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "C", "new_string": "C\nD" }),
        )
        .await;
        assert!(!r1.is_error, "first edit failed: {:?}", r1.content);
        let r2 = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "A", "new_string": "A2" }),
        )
        .await;
        assert!(!r2.is_error, "second edit failed: {:?}", r2.content);
        assert_eq!(disk(&ctx, &name), "A2\nB\nC\nD");
    }

    #[tokio::test]
    async fn edit_matches_curly_quotes_against_ascii_file() {
        let (ctx, name) = seed("他说\"你好\"。");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({
                "path": name,
                "old_string": "他说\u{201C}你好\u{201D}。",
                "new_string": "他说\u{201C}再见\u{201D}。",
            }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "他说\"再见\"。");
        assert_eq!(res.content["old_string"], "他说\"你好\"。");
        assert_eq!(res.content["new_string"], "他说\"再见\"。");
    }

    #[tokio::test]
    async fn edit_matches_ascii_quotes_against_corner_brackets() {
        let (ctx, name) = seed("他说「你好」。");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({
                "path": name,
                "old_string": "他说\"你好\"。",
                "new_string": "他说\"再见\"。",
            }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "他说「再见」。");
    }

    #[tokio::test]
    async fn edit_prefers_exact_match_when_quote_styles_coexist() {
        let (ctx, name) = seed("ascii: \"你好\"\ncurly: \u{201C}你好\u{201D}\n");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({
                "path": name,
                "old_string": "\u{201C}你好\u{201D}",
                "new_string": "OK",
            }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "ascii: \"你好\"\ncurly: OK\n");
    }

    #[tokio::test]
    async fn edit_quote_fold_replace_all() {
        let (ctx, name) = seed("\"A\" and \"A\"");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({
                "path": name,
                "old_string": "\u{201C}A\u{201D}",
                "new_string": "\u{201C}B\u{201D}",
                "replace_all": true,
            }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "\"B\" and \"B\"");
        assert_eq!(res.content["replaced_count"], 2);
    }

    #[tokio::test]
    async fn edit_matches_lf_locator_against_crlf_file() {
        let (ctx, name) = seed("A\r\nB\r\nC");
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "A\nB", "new_string": "X\nY" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "X\r\nY\r\nC");
    }

    #[tokio::test]
    async fn edit_anchor_span_when_middle_is_paraphrased() {
        let start = "START_ANCHOR_UNIQUE_24CH!!";
        let end = "END_ANCHOR_UNIQUE_24CHARS!!";
        let file_mid = "the original passage sits here and is several sentences long so the locator is over eighty characters.";
        let old_mid = "the rewritten passage sits here and is several sentences long so the locator is over eighty characters.";
        let (ctx, name) = seed(&format!("intro\n{start}{file_mid}{end}\noutro"));
        let res = run_edit(
            &ctx,
            json!({
                "path": name,
                "old_string": format!("{start}{old_mid}{end}"),
                "new_string": format!("{start}EXPANDED{end}"),
            }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), format!("intro\n{start}EXPANDED{end}\noutro"));
    }

    #[tokio::test]
    async fn edit_anchor_span_rejects_ambiguous_head() {
        let start = "START_ANCHOR_UNIQUE_24CH!!";
        let end = "END_ANCHOR_UNIQUE_24CHARS!!";
        let pad = "the rewritten passage sits here and is several sentences long so the locator is over eighty characters.";
        let (ctx, name) = seed(&format!("{start}aaa{end}\n{start}bbb{end}"));
        let res = run_edit(
            &ctx,
            json!({
                "path": name,
                "old_string": format!("{start}{pad}{end}"),
                "new_string": "NO",
            }),
        )
        .await;
        assert!(res.is_error);
        assert_eq!(disk(&ctx, &name), format!("{start}aaa{end}\n{start}bbb{end}"));
    }
}
