//! File-mutation tools: `Write` (overwrite) and `Edit` (paragraph-range replace).
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
//! `Edit` has one operation: replace an inclusive paragraph range with new
//! content. The range comes from a single `from` argument that accepts a
//! paragraph number (`5`), a range (`"1-9"`, `"1~9"`), or a contiguous
//! enumeration (`"1,2,3"`). An empty `content` deletes the range (it is just a
//! replacement with nothing). Appending / continuing prose is expressed as
//! replacing the LAST paragraph with `content` that begins with that
//! paragraph's existing text and then continues.
//!
//! Paragraph numbers come from the `[P001]` labels returned by Read. Because
//! edits shift later numbers, `Edit`:
//!
//! - verifies the on-disk content still matches the receipt hash recorded at
//!   the last Read/Write (stale files are rejected, not silently mis-edited);
//! - updates the read receipt after every successful edit so consecutive edits
//!   remain safe.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{FileOp, FileSnapshotStore};
use crate::ai::agent::tools::paragraph::{
    join_paragraphs, parse_paragraph_spec, split_agent_paragraphs, split_paragraphs,
    strip_paragraph_label,
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
                description: "Replace a numbered paragraph range in a file. Read the file first to get `[P001]`, … \
                    labels (one line = one paragraph). Pass `path`, `from`, and `content`. `from` locates the \
                    paragraph(s) to replace: a single number (`5`), a range (`\"1-9\"` or `\"1~9\"`), or a contiguous \
                    enumeration (`\"1,2,3,4\"`). `content` is the complete replacement text (use \\n between \
                    paragraphs). To DELETE, pass empty `content`. To CONTINUE/APPEND after the last paragraph, set \
                    `from` to that last paragraph number and make `content` start with its existing text, then add the \
                    new prose (e.g. last paragraph is `哦哦哦` → content `哦哦哦。后续新内容`). Do NOT copy other \
                    unaffected paragraphs into `content`. Paragraph numbers SHIFT after edits; for several edits to \
                    one file, work from the BOTTOM up."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": FILE_REF_DESC
                        },
                        "from": {
                            "type": ["integer", "string"],
                            "description": "Paragraph(s) to replace (1-based, inclusive). A number like 5, a range like \"1-9\" / \"1~9\", or a contiguous enumeration like \"1,2,3,4\". To continue past the end, set this to the LAST paragraph number and lead `content` with its existing text."
                        },
                        "content": {
                            "type": "string",
                            "description": "Replacement text; use \\n between paragraphs. Empty string deletes the range. When continuing/appending, begin with the last paragraph's existing text then add the new prose. Fill this in LAST, after path/from."
                        }
                    },
                    "required": ["path", "from", "content"]
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
        parse_paragraph_spec(input.get("from"))?;
        require_string(input, "content", EDIT_TOOL)?;
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let path = path_arg(&invocation.input, EDIT_TOOL, &invocation.context.cwd)?;
            let range = parse_paragraph_spec(invocation.input.get("from"))?;
            let content = normalize_tool_string(strip_paragraph_label(
                invocation
                    .input
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ));

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

            let from = range.from;
            let to = range.to;
            if from < 1 || from > total_before || to > total_before {
                return Ok(range_error(from, to, total_before));
            }

            let from_idx = from - 1;
            let before = paragraphs[from_idx..to].join("\n");

            let new_paragraph_count = if content.is_empty() {
                paragraphs.drain(from_idx..to);
                0
            } else {
                let new_paragraphs = split_agent_paragraphs(&content);
                let count = new_paragraphs.len();
                paragraphs.splice(from_idx..to, new_paragraphs);
                count
            };

            let updated = join_paragraphs(&paragraphs);
            let new_paragraph_to = if new_paragraph_count == 0 {
                from - 1
            } else {
                from + new_paragraph_count - 1
            };

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
                "from": from,
                "replaced_from": from,
                "replaced_to": to,
                "new_paragraph_to": new_paragraph_to,
                "total_paragraphs": paragraphs.len(),
                "before": before,
            })))
        })
    }
}

fn range_error(from: usize, to: usize, total: usize) -> ToolResult {
    ToolResult::error(format!(
        "Edit: `from` {from}-{to} out of range (file has {total} paragraphs). Read the file again."
    ))
}

fn stale_error(path: &Path) -> ToolResult {
    ToolResult::error(format!(
        "Edit: {} changed since you last read it — Read the file again before editing.",
        path.display()
    ))
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
    async fn replaces_dash_range_with_multiple_paragraphs() {
        let (ctx, name) = seed("A\nB\nC\nD");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "from": "2-3", "content": "X\nY\nZ" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nX\nY\nZ\nD");
        assert_eq!(res.content["success"], true);
        assert_eq!(res.content["from"], 2);
        assert_eq!(res.content["replaced_from"], 2);
        assert_eq!(res.content["replaced_to"], 3);
        assert_eq!(res.content["new_paragraph_to"], 4);
        assert_eq!(res.content["total_paragraphs"], 5);
        assert_eq!(res.content["before"], "B\nC");
    }

    #[tokio::test]
    async fn replaces_contiguous_enumeration() {
        let (ctx, name) = seed("A\nB\nC\nD");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "from": "1,2,3", "content": "X" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "X\nD");
        assert_eq!(res.content["replaced_from"], 1);
        assert_eq!(res.content["replaced_to"], 3);
        assert_eq!(res.content["before"], "A\nB\nC");
    }

    #[tokio::test]
    async fn integer_from_replaces_exactly_one_paragraph() {
        let (ctx, name) = seed("A\nB\nC\nD");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "from": 2, "content": "X" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nX\nC\nD");
    }

    #[tokio::test]
    async fn continue_by_replacing_last_paragraph_with_its_text_plus_new() {
        let (ctx, name) = seed("A\nB\n哦哦哦");
        read_receipt(&ctx, &name).await;
        // Continuation is expressed as replacing the last paragraph (3) with
        // its existing text followed by the new prose.
        let res = run_edit(
            &ctx,
            json!({ "path": name, "from": 3, "content": "哦哦哦。后续新内容\n再一段" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nB\n哦哦哦。后续新内容\n再一段");
        assert_eq!(res.content["new_paragraph_to"], 4);
        assert_eq!(res.content["before"], "哦哦哦");
    }

    #[tokio::test]
    async fn empty_content_deletes_range_without_blank_residue() {
        let (ctx, name) = seed("A\nB\nC\nD");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "from": "2-3", "content": "" }),
        )
        .await;
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(disk(&ctx, &name), "A\nD");
        assert_eq!(res.content["new_paragraph_to"], 1);
        assert_eq!(res.content["before"], "B\nC");
    }

    #[tokio::test]
    async fn rejects_non_contiguous_enumeration() {
        let (ctx, name) = seed("A\nB\nC\nD");
        read_receipt(&ctx, &name).await;
        let tool = FileEditTool::new(Arc::new(FileSnapshotStore::new()));
        let input = json!({ "path": name, "from": "1,3", "content": "X" });
        assert!(tool.validate(&input).is_err());
        // Nothing written.
        assert_eq!(disk(&ctx, &name), "A\nB\nC\nD");
    }

    #[tokio::test]
    async fn rejects_out_of_range_paragraph() {
        let (ctx, name) = seed("A\nB");
        read_receipt(&ctx, &name).await;
        let res = run_edit(
            &ctx,
            json!({ "path": name, "from": 4, "content": "X" }),
        )
        .await;
        assert!(res.is_error);
        assert_eq!(disk(&ctx, &name), "A\nB");
    }

    #[tokio::test]
    async fn stale_file_is_rejected() {
        let (ctx, name) = seed("A\nB\nC");
        read_receipt(&ctx, &name).await;
        // Simulate an out-of-band change (user edited in the reader, etc.).
        std::fs::write(ctx.cwd.join(&name), "A\nB\nC\nD\nE").unwrap();
        let res = run_edit(
            &ctx,
            json!({ "path": name, "from": 2, "content": "Z" }),
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
            json!({ "path": name, "from": 1, "content": "Z" }),
        )
        .await;
        assert!(res.is_error);
    }

    #[tokio::test]
    async fn consecutive_edits_use_refreshed_receipt() {
        let (ctx, name) = seed("A\nB\nC");
        read_receipt(&ctx, &name).await;
        // Continue by replacing the last paragraph (3) with its text + new.
        let r1 = run_edit(
            &ctx,
            json!({ "path": name, "from": 3, "content": "C\nD" }),
        )
        .await;
        assert!(!r1.is_error, "first edit failed: {:?}", r1.content);
        // Second edit in the same session must not be rejected as stale.
        let r2 = run_edit(
            &ctx,
            json!({ "path": name, "from": 1, "content": "A2" }),
        )
        .await;
        assert!(!r2.is_error, "second edit rejected: {:?}", r2.content);
        assert_eq!(disk(&ctx, &name), "A2\nB\nC\nD");
    }
}
