//! File-mutation tools: `Write` (overwrite) and `Edit` (string replace).
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
//! `Edit` has one operation: find an exact `old_string` in the file and
//! replace it with `new_string`. `old_string` must be copied verbatim from a
//! prior Read (Read returns plain, unlabeled text). By default `old_string`
//! must match exactly once; if it occurs multiple times the edit is rejected
//! unless `replace_all` is set, in which case every occurrence is replaced. An
//! empty `new_string` deletes the matched text. Because the match is
//! content-based (not positional), `Edit`:
//!
//! - verifies the on-disk content still matches the receipt hash recorded at
//!   the last Read/Write (stale files are rejected, not silently mis-edited);
//! - updates the read receipt after every successful edit so consecutive edits
//!   remain safe.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{FileOp, FileSnapshotStore};
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
                description: "Replace an exact substring in a file. Read the file first (Read returns plain text). \
                    Pass `path`, `old_string`, and `new_string`. `old_string` is text copied VERBATIM from the file \
                    (including whitespace and line breaks) and must be long enough to match EXACTLY ONE place — \
                    include surrounding context to disambiguate. `new_string` is what replaces it. To DELETE, pass an \
                    empty `new_string`. To CONTINUE/APPEND after existing prose, set `old_string` to the tail of the \
                    current text and make `new_string` begin with that same text, then add the new prose (e.g. the \
                    file ends with `哦哦哦` → old_string `哦哦哦`, new_string `哦哦哦。后续新内容`). If `old_string` \
                    intentionally appears multiple times and you want to replace every occurrence, set `replace_all` \
                    to true; otherwise a non-unique match is rejected. If Edit fails (not found, not unique, or file \
                    changed), Read the file again before retrying."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": FILE_REF_DESC
                        },
                        "old_string": {
                            "type": "string",
                            "description": "Exact text to replace, copied verbatim from the file (whitespace and line breaks included). Must match exactly once unless `replace_all` is true. Include enough surrounding context to be unique."
                        },
                        "new_string": {
                            "type": "string",
                            "description": "Replacement text. Empty string deletes `old_string`. When continuing/appending, begin with `old_string`'s existing text then add the new prose. Fill this in LAST, after path/old_string."
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
            let old_string = normalize_tool_string(
                invocation
                    .input
                    .get("old_string")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            let new_string = normalize_tool_string(
                invocation
                    .input
                    .get("new_string")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            );
            let replace_all = invocation
                .input
                .get("replace_all")
                .and_then(Value::as_bool)
                .unwrap_or(false);

            if old_string.is_empty() {
                return Ok(ToolResult::error(
                    "Edit: `old_string` must be non-empty".to_string(),
                ));
            }
            if old_string == new_string {
                return Ok(ToolResult::error(
                    "Edit: `old_string` and `new_string` are identical — nothing to change"
                        .to_string(),
                ));
            }

            let stored_hash = lookup_receipt(&invocation.context.read_file_state, &path);
            if stored_hash.is_none() {
                return Ok(ToolResult::error(format!(
                    "Edit: read {} first — no receipt on this session",
                    path.display()
                )));
            }

            let decoded = read_text_file(&path)
                .map_err(|e| AppError::Other(format!("Edit: read {:?}: {e}", path)))?;

            // Stale-file guard: the on-disk content must still match what the
            // model last saw. If it drifted (user edited in the reader, a
            // rejected diff was written back, etc.), refuse so the model
            // re-reads instead of editing the wrong text.
            let disk_hash = content_hash(&decoded.text);
            if stored_hash != Some(disk_hash) {
                return Ok(stale_error(&path));
            }

            let occurrences = decoded.text.matches(&old_string).count();
            if occurrences == 0 {
                return Ok(not_found_error(&path));
            }
            if occurrences > 1 && !replace_all {
                return Ok(not_unique_error(occurrences));
            }

            let match_start = decoded
                .text
                .find(&old_string)
                .map(|byte_idx| decoded.text[..byte_idx].chars().count())
                .unwrap_or(0);

            let text_before = decoded.text.clone();
            let (updated, replaced_count) = if replace_all {
                (decoded.text.replace(&old_string, &new_string), occurrences)
            } else {
                (decoded.text.replacen(&old_string, &new_string, 1), 1)
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
                "old_string": old_string,
                "new_string": new_string,
                "replace_all": replace_all,
                "replaced_count": replaced_count,
                "match_start": match_start,
                "text_before": text_before,
                "text": updated,
            })))
        })
    }
}

fn not_found_error(path: &Path) -> ToolResult {
    ToolResult::error(format!(
        "Edit: `old_string` not found in {}. Read the file again and copy the exact text.",
        path.display()
    ))
}

fn not_unique_error(occurrences: usize) -> ToolResult {
    ToolResult::error(format!(
        "Edit: `old_string` matched {occurrences} places — add more surrounding context to make it unique, or set `replace_all` to true."
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
        assert_eq!(res.content["text_before"], "A\nB\nC\nD");
        assert_eq!(res.content["text"], "A\nX\nY\nZ\nD");
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
    async fn stale_file_is_rejected() {
        let (ctx, name) = seed("A\nB\nC");
        read_receipt(&ctx, &name).await;
        // Simulate an out-of-band change (user edited in the reader, etc.).
        std::fs::write(ctx.cwd.join(&name), "A\nB\nC\nD\nE").unwrap();
        let res = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "B", "new_string": "Z" }),
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
            json!({ "path": name, "old_string": "A", "new_string": "Z" }),
        )
        .await;
        assert!(res.is_error);
    }

    #[tokio::test]
    async fn consecutive_edits_use_refreshed_receipt() {
        let (ctx, name) = seed("A\nB\nC");
        read_receipt(&ctx, &name).await;
        // Continue by replacing the tail with its text + new.
        let r1 = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "C", "new_string": "C\nD" }),
        )
        .await;
        assert!(!r1.is_error, "first edit failed: {:?}", r1.content);
        // Second edit in the same session must not be rejected as stale.
        let r2 = run_edit(
            &ctx,
            json!({ "path": name, "old_string": "A", "new_string": "A2" }),
        )
        .await;
        assert!(!r2.is_error, "second edit rejected: {:?}", r2.content);
        assert_eq!(disk(&ctx, &name), "A2\nB\nC\nD");
    }
}
