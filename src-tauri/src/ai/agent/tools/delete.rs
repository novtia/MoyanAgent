//! `Delete` — remove a file, capturing a pre-image for snapshot rollback.
//!
//! Deletion previously had no dedicated tool (the model had to shell out via
//! `Bash`), which meant the file-snapshot system couldn't capture it. This
//! tool records the file's pre-image into the shared [`FileSnapshotStore`]
//! before unlinking it, so deleting / regenerating the triggering message can
//! recreate the file.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{FileOp, FileSnapshotStore};
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "Delete";

#[derive(Clone)]
pub struct DeleteTool {
    spec: ToolSpec,
    snapshots: Arc<FileSnapshotStore>,
}

impl DeleteTool {
    pub fn new(snapshots: Arc<FileSnapshotStore>) -> Self {
        Self {
            snapshots,
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Delete a file from disk. Requires an absolute path to an existing \
                    regular file. The deletion is snapshotted so it can be rolled back if the \
                    triggering message is later removed. Prefer this over a Bash `rm` so the \
                    change is tracked."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the file to delete."
                        }
                    },
                    "required": ["path"]
                }),
                read_only: false,
                concurrency_safe: false,
            },
        }
    }
}

impl Tool for DeleteTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let raw = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid(format!("{TOOL_NAME}: `path` must be a string")))?;
        if raw.trim().is_empty() {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `path` must be non-empty"
            )));
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let raw = invocation
                .input
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let path = PathBuf::from(raw);
            if !path.is_absolute() {
                return Err(AppError::Invalid(format!(
                    "{TOOL_NAME}: `path` must be absolute, got `{raw}`"
                )));
            }
            if !path.exists() {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: file does not exist: {}",
                    path.display()
                )));
            }
            if !path.is_file() {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: not a regular file: {}",
                    path.display()
                )));
            }

            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();

            // Snapshot the pre-image before unlinking so a rollback can
            // recreate the file with its original content.
            self.snapshots.record_before(
                invocation.context.session_id.as_deref(),
                &path,
                FileOp::Delete,
            );

            std::fs::remove_file(&path)
                .map_err(|e| AppError::Other(format!("{TOOL_NAME}: remove {:?}: {e}", path)))?;

            Ok(ToolResult::ok(json!({
                "path": path.to_string_lossy(),
                "name": name,
                "deleted": true,
            })))
        })
    }
}
