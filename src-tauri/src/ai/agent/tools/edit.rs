//! File-mutation tools: `Write` (overwrite) and `Edit` (paragraph-range edit).
//!
//! Both refuse to operate on a file that the agent hasn't read first,
//! using [`crate::ai::agent::core::context::ToolUseContext::read_file_state`]
//! as the receipt. This mirrors the safety property in the TS executor
//! and prevents the model from clobbering files it doesn't actually
//! understand.
//!
//! The `Read` precondition is skipped for `Write` when the target file
//! does not yet exist — creating new files is always fine.
//!
//! `Edit` is paragraph-number addressed (`[P001]` labels from Read), which is
//! cheap but fragile: every insert/delete shifts the numbers of the paragraphs
//! below it. To keep the model from editing the wrong place after such a shift,
//! `Edit`:
//!
//! - verifies the on-disk content still matches the receipt hash recorded at
//!   the last Read/Write (stale files are rejected, not silently mis-edited);
//! - optionally verifies an `anchor` snippet against the target paragraph;
//! - supports `replace` / `insert_after` / `delete` modes so appending no
//!   longer requires copying an existing paragraph back into `content`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{FileOp, FileSnapshotStore};
use crate::ai::agent::tools::paragraph::{
    join_paragraphs, split_agent_paragraphs, split_paragraphs, strip_paragraph_label,
};
use crate::ai::agent::tools::project_path::{self, FILE_REF_DESC};
use crate::ai::agent::tools::read_receipt::{
    content_hash, has_receipt, lookup_receipt, record_receipt,
};
use crate::ai::agent::tools::text_decode::{
    detect_and_decode, normalize_tool_string, read_text_file, write_text_file, TextEncoding,
};
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const WRITE_TOOL: &str = "Write";
const EDIT_TOOL: &str = "Edit";

/// How an [`FileEditTool`] call mutates the target paragraph range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EditMode {
    /// Replace `[paragraph_from, paragraph_to]` with `content`.
    Replace,
    /// Insert `content` *after* `paragraph_from` (`0` = file start).
    InsertAfter,
    /// Delete `[paragraph_from, paragraph_to]` entirely (no empty residue).
    Delete,
}

impl EditMode {
    fn parse(input: &Value) -> Self {
        match input.get("mode").and_then(Value::as_str) {
            Some("insert_after") | Some("insert") | Some("append_after") => EditMode::InsertAfter,
            Some("delete") | Some("remove") => EditMode::Delete,
            _ => EditMode::Replace,
        }
    }

    fn as_str(&self) -> &'static str {
        match self {
            EditMode::Replace => "replace",
            EditMode::InsertAfter => "insert_after",
            EditMode::Delete => "delete",
        }
    }
}

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
            let path = path_arg(&invocation.input, WRITE_TOOL, &invocation.context.cwd)?;
            let content = normalize_tool_string(
                invocation
                    .input
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );

            let exists = path.exists();
            if exists && !has_receipt(&invocation.context.read_file_state, &path) {
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

            record_receipt(&invocation.context.read_file_state, &path, &content);

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
                description: "Edit a numbered paragraph range in a file. Read labels lines `[P001]`, …; one line = one paragraph. \
                    Read the file first, then pass `paragraph_from`, optional `paragraph_to` (defaults to `paragraph_from`), \
                    `content`, and optional `mode`. \
                    IMPORTANT: paragraph numbers SHIFT after every insert/delete — do NOT reuse the old numbers. When making several edits \
                    to one file, edit from the BOTTOM up (largest paragraph numbers first) so earlier numbers stay valid. \
                    Pass `anchor` (the first few characters of `paragraph_from`) to have the edit verified against the \
                    current text and rejected if it no longer matches. Modes: `replace` (default) swaps the range for \
                    `content`; `insert_after` inserts `content` after `paragraph_from` (use 0 for the file start) without \
                    touching existing text — use this to APPEND, never replace-and-recopy; `delete` removes the range."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": FILE_REF_DESC
                        },
                        "mode": {
                            "type": "string",
                            "enum": ["replace", "insert_after", "delete"],
                            "description": "`replace` (default): swap the range for `content`. `insert_after`: insert `content` after `paragraph_from` (0 = file start); nothing is removed. `delete`: remove the range."
                        },
                        "paragraph_from": {
                            "type": "integer",
                            "minimum": 0,
                            "description": "First paragraph to edit (1-based, inclusive). E.g. [P009] → 9. For `insert_after`, this is the paragraph to insert AFTER (0 = insert at the very start)."
                        },
                        "paragraph_to": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Last paragraph to edit (1-based, inclusive). Defaults to `paragraph_from`. Ignored for `insert_after`."
                        },
                        "anchor": {
                            "type": "string",
                            "description": "Optional. The first few characters (≈8–16) of `paragraph_from` as you last saw it. If it does not match the current text, the edit is rejected — Read the file again before retrying."
                        },
                        "content": {
                            "type": "string",
                            "description": "New text. \\n-separated for multiple paragraphs. Required for `replace`/`insert_after`; ignored for `delete`. Empty string in `replace` mode deletes the range. Fill this in LAST, after path/mode/paragraph_from/paragraph_to/anchor."
                        }
                    },
                    "required": ["path", "paragraph_from"]
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
        let mode = EditMode::parse(input);
        let from = parse_paragraph(input.get("paragraph_from"), "paragraph_from")?
            .ok_or_else(|| AppError::Invalid("Edit: `paragraph_from` is required".into()))?;
        // `insert_after` allows 0 (insert at the very start); every other mode
        // addresses an existing paragraph and must be >= 1.
        if from == 0 && mode != EditMode::InsertAfter {
            return Err(AppError::Invalid(
                "Edit: `paragraph_from` must be >= 1".into(),
            ));
        }
        if let Some(to) = parse_paragraph(input.get("paragraph_to"), "paragraph_to")? {
            if to == 0 {
                return Err(AppError::Invalid(
                    "Edit: `paragraph_to` must be >= 1".into(),
                ));
            }
            if to < from {
                return Err(AppError::Invalid(format!(
                    "Edit: `paragraph_to` ({to}) must be >= `paragraph_from` ({from})"
                )));
            }
        }
        // `delete` ignores `content`; the other modes need a string when present.
        if mode != EditMode::Delete && input.get("content").is_some() {
            require_string(input, "content", EDIT_TOOL)?;
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = path_arg(&invocation.input, EDIT_TOOL, &invocation.context.cwd)?;
            let mode = EditMode::parse(&invocation.input);
            let paragraph_from = parse_paragraph(
                invocation.input.get("paragraph_from"),
                "paragraph_from",
            )?
            .unwrap_or(0);
            let paragraph_to = parse_paragraph(
                invocation.input.get("paragraph_to"),
                "paragraph_to",
            )?
            .unwrap_or(paragraph_from);
            let content = normalize_tool_string(strip_paragraph_label(
                invocation
                    .input
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ));
            let anchor = invocation
                .input
                .get("anchor")
                .and_then(Value::as_str)
                .map(|a| normalize_tool_string(strip_paragraph_label(a)));

            let stored_hash = lookup_receipt(&invocation.context.read_file_state, &path);
            if stored_hash.is_none() {
                return Ok(ToolResult::error(format!(
                    "Edit: read {} first — no receipt on this session",
                    path.display()
                )));
            }

            let decoded = read_text_file(&path)
                .map_err(|e| AppError::Other(format!("Edit: read {:?}: {e}", path)))?;
            let mut paragraphs = split_paragraphs(&decoded.text);
            let total_before = paragraphs.len();

            // Stale-file guard: the on-disk content must still match what the
            // model last saw. If it drifted (user edited in the reader, a
            // rejected diff was written back, etc.), refuse so the model
            // re-reads instead of editing the wrong paragraph.
            let disk_hash = content_hash(&decoded.text);
            if stored_hash != Some(disk_hash) {
                return Ok(stale_error(&path));
            }

            // Range / bounds validation, mode-aware.
            match mode {
                EditMode::InsertAfter => {
                    // `paragraph_from` is an "insert after" anchor: 0 = start,
                    // N = after paragraph N. It may equal the paragraph count.
                    if paragraph_from > total_before {
                        return Ok(range_error("paragraph_from", paragraph_from, total_before));
                    }
                }
                EditMode::Replace | EditMode::Delete => {
                    if paragraph_from == 0 || paragraph_from > total_before {
                        return Ok(range_error("paragraph_from", paragraph_from, total_before));
                    }
                    if paragraph_to < paragraph_from || paragraph_to > total_before {
                        return Ok(range_error("paragraph_to", paragraph_to, total_before));
                    }
                }
            }

            // Anchor verification (skipped for insert-at-start, which has no
            // target paragraph).
            if let Some(anchor) = anchor.as_deref() {
                let target_idx = match mode {
                    EditMode::InsertAfter if paragraph_from == 0 => None,
                    _ => Some(paragraph_from - 1),
                };
                if let Some(idx) = target_idx {
                    if !anchor_matches(&paragraphs[idx], anchor) {
                        return Ok(anchor_error(&path, paragraph_from));
                    }
                }
            }

            // Determine the effective mode: an empty `content` in `replace`
            // means "delete the range" so no stray blank paragraph is left.
            let content_is_blank =
                split_agent_paragraphs(&content).iter().all(|p| p.is_empty());
            let effective_mode = if mode == EditMode::Replace && content_is_blank {
                EditMode::Delete
            } else {
                mode
            };

            match effective_mode {
                EditMode::Replace => {
                    let from_idx = paragraph_from - 1;
                    let to_idx = paragraph_to - 1;
                    let new_paras = split_agent_paragraphs(&content);
                    paragraphs.splice(
                        from_idx..=to_idx,
                        if new_paras.is_empty() {
                            vec![String::new()]
                        } else {
                            new_paras
                        },
                    );
                }
                EditMode::Delete => {
                    let from_idx = paragraph_from - 1;
                    let to_idx = paragraph_to - 1;
                    paragraphs.drain(from_idx..=to_idx);
                }
                EditMode::InsertAfter => {
                    let new_paras = split_agent_paragraphs(&content);
                    if new_paras.is_empty() || new_paras.iter().all(|p| p.is_empty()) {
                        return Ok(ToolResult::error(
                            "Edit: `insert_after` needs non-empty `content`".to_string(),
                        ));
                    }
                    let insert_at = paragraph_from; // 0 = start, N = after paragraph N
                    for (offset, p) in new_paras.into_iter().enumerate() {
                        paragraphs.insert(insert_at + offset, p);
                    }
                }
            };

            let updated = join_paragraphs(&paragraphs);

            // Snapshot the pre-image before mutating for rollback support.
            self.snapshots.record_before(
                invocation.context.session_id.as_deref(),
                &path,
                FileOp::Update,
            );

            write_text_file(&path, &updated, decoded.encoding, decoded.had_bom)
                .map_err(|e| AppError::Other(format!("Edit: write {:?}: {e}", path)))?;

            record_receipt(&invocation.context.read_file_state, &path, &updated);

            Ok(ToolResult::ok(json!({
                "success": true,
                "path": path.to_string_lossy(),
                "paragraph_from": paragraph_from,
                "paragraph_to": paragraph_to,
            })))
        })
    }
}

/// Does the target paragraph still begin with the model-supplied `anchor`?
/// Tolerant of leading whitespace and short paragraphs.
fn anchor_matches(paragraph: &str, anchor: &str) -> bool {
    let p = strip_paragraph_label(paragraph).trim();
    let a = anchor.trim();
    if a.is_empty() {
        return true;
    }
    p.starts_with(a) || a.starts_with(p)
}

fn range_error(field: &str, value: usize, total: usize) -> ToolResult {
    ToolResult::error(format!(
        "Edit: `{field}` {value} out of range (file has {total} paragraphs). Read the file again."
    ))
}

fn stale_error(path: &Path) -> ToolResult {
    ToolResult::error(format!(
        "Edit: {} changed since you last read it — Read the file again before editing.",
        path.display()
    ))
}

fn anchor_error(path: &Path, paragraph_from: usize) -> ToolResult {
    ToolResult::error(format!(
        "Edit: anchor did not match paragraph {paragraph_from} of {} — Read the file again.",
        path.display()
    ))
}

fn parse_paragraph(v: Option<&Value>, key: &str) -> AppResult<Option<usize>> {
    match v {
        None => Ok(None),
        Some(Value::Null) => Ok(None),
        Some(n) => {
            let n = n.as_u64().ok_or_else(|| {
                AppError::Invalid(format!("Edit: `{key}` must be a positive integer"))
            })?;
            Ok(Some(n as usize))
        }
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

fn path_arg(input: &Value, tool: &str, cwd: &Path) -> AppResult<PathBuf> {
    let raw = input
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::Invalid(format!("{tool}: missing path")))?;
    project_path::resolve_project_file(cwd, raw, tool)
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

    async fn run_edit(ctx: &Arc<ToolUseContext>, mut input: Value) -> ToolResult {
        if input.get("path").is_none() {
            panic!("edit test input needs a path");
        }
        // Ensure serde_json object.
        let obj = input.as_object_mut().unwrap();
        obj.entry("mode").or_insert(json!("replace"));
        let tool = FileEditTool::new(Arc::new(FileSnapshotStore::new()));
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
    async fn replace_first_paragraph_multiline_overwrites_in_place() {
        let (ctx, name) = seed("你好。\n结尾");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "paragraph_from": 1, "content": "你好。\n新世界。" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "你好。\n新世界。\n结尾");
        assert_eq!(res.content["success"], true);
        assert_eq!(res.content["paragraph_from"], 1);
    }

    #[tokio::test]
    async fn insert_after_appends_without_touching_existing() {
        let (ctx, name) = seed("A\nB");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "mode": "insert_after", "paragraph_from": 1, "content": "X" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nX\nB");
        assert_eq!(res.content["success"], true);
        assert_eq!(res.content["paragraph_from"], 1);
    }

    #[tokio::test]
    async fn insert_after_zero_prepends_at_start() {
        let (ctx, name) = seed("A\nB");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "mode": "insert_after", "paragraph_from": 0, "content": "X\nY" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "X\nY\nA\nB");
    }

    #[tokio::test]
    async fn delete_removes_range_without_blank_residue() {
        let (ctx, name) = seed("A\nB\nC");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "mode": "delete", "paragraph_from": 2, "paragraph_to": 2 }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nC");
        assert_eq!(res.content["success"], true);
    }

    #[tokio::test]
    async fn replace_with_empty_content_deletes_range() {
        let (ctx, name) = seed("A\nB\nC");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "paragraph_from": 2, "content": "" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nC");
        assert_eq!(res.content["success"], true);
    }

    #[tokio::test]
    async fn anchor_mismatch_is_rejected() {
        let (ctx, name) = seed("A\nB\nC\nD\nE");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "paragraph_from": 2, "content": "Z", "anchor": "totally-wrong" }),
        )
        .await;
        assert!(res.is_error);
        assert!(res.content.get("error").is_some());
        assert!(res.content.get("window").is_none());
        // File untouched.
        assert_eq!(disk(&ctx, &name), "A\nB\nC\nD\nE");
    }

    #[tokio::test]
    async fn anchor_match_allows_edit() {
        let (ctx, name) = seed("你好世界\nB");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "paragraph_from": 1, "content": "改写", "anchor": "你好" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "改写\nB");
    }

    #[tokio::test]
    async fn stale_file_is_rejected() {
        let (ctx, name) = seed("A\nB\nC");
        read_receipt(&ctx, &name).await;
        // Simulate an out-of-band change (user edited in the reader, etc.).
        std::fs::write(ctx.cwd.join(&name), "A\nB\nC\nD\nE").unwrap();
        let res = run_edit(
            &ctx,
            json!({ "path": name, "paragraph_from": 2, "content": "Z" }),
        )
        .await;
        assert!(res.is_error);
        assert!(res.content.get("error").is_some());
        assert!(res.content.get("window").is_none());
        // The stale edit must not have been applied.
        assert_eq!(disk(&ctx, &name), "A\nB\nC\nD\nE");
    }

    #[tokio::test]
    async fn edit_without_receipt_errors() {
        let (ctx, name) = seed("A\nB");
        // No read_receipt call → no receipt.
        let res = run_edit(
            &ctx,
            json!({ "path": name, "paragraph_from": 1, "content": "Z" }),
        )
        .await;
        assert!(res.is_error);
    }

    #[tokio::test]
    async fn consecutive_edits_use_refreshed_receipt() {
        let (ctx, name) = seed("A\nB\nC");
        read_receipt(&ctx, &name).await;
        let r1 = run_edit(
            &ctx,
            json!({ "path": name, "mode": "insert_after", "paragraph_from": 3, "content": "D" }),
        )
        .await;
        assert!(!r1.is_error, "first edit failed: {:?}", r1.content);
        // Second edit in the same session must not be rejected as stale.
        let r2 = run_edit(
            &ctx,
            json!({ "path": name, "paragraph_from": 1, "content": "A2" }),
        )
        .await;
        assert!(!r2.is_error, "second edit rejected: {:?}", r2.content);
        assert_eq!(disk(&ctx, &name), "A2\nB\nC\nD");
    }
}
