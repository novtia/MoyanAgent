//! `ListFiles` — enumerate files and subdirectories under a path.
//!
//! Always returns a fully nested tree: every directory node includes a `children`
//! array (possibly empty) with its files and subfolders inside.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "ListFiles";
const DEFAULT_MAX_ENTRIES: usize = 500;
const MAX_ENTRIES_CAP: usize = 5_000;

#[derive(Clone, Serialize)]
struct ListEntry {
    name: String,
    kind: &'static str,
    /// Present on every `directory` node (may be `[]`). Omitted on `file` nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<ListEntry>>,
}

#[derive(Clone)]
pub struct ListFilesTool {
    spec: ToolSpec,
}

impl Default for ListFilesTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ListFilesTool {
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "List a directory as a fully nested tree. \
                    Returns `{ success, entries: [{ name, kind, children? }] }` where \
                    each directory has `children: [...]` containing its files and \
                    subfolders (recursively). `kind` is `directory` or `file`. \
                    Use instead of Bash `dir`/`ls` for reliable Unicode paths."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the directory to list."
                        },
                        "max_entries": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": MAX_ENTRIES_CAP,
                            "default": DEFAULT_MAX_ENTRIES,
                            "description": "Stop after this many nodes total (safety cap for huge trees)."
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

impl Tool for ListFilesTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let path = input
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid(format!("{TOOL_NAME}: `path` must be a string")))?;
        if path.trim().is_empty() {
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
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: `path` must be absolute, got `{raw}`"
                )));
            }

            let max_entries = invocation
                .input
                .get("max_entries")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_ENTRIES)
                .clamp(1, MAX_ENTRIES_CAP);

            let canonical = std::fs::canonicalize(&path).map_err(|e| {
                AppError::Other(format!("{TOOL_NAME}: canonicalize {:?}: {e}", path))
            })?;
            if !canonical.is_dir() {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: not a directory: {}",
                    canonical.display()
                )));
            }

            let mut count = 0usize;
            let mut truncated = false;
            let entries = collect_tree(&canonical, max_entries, &mut count, &mut truncated)?;

            Ok(ToolResult::ok(json!({
                "success": true,
                "path": canonical.to_string_lossy(),
                "truncated": truncated,
                "entries": entries,
            })))
        })
    }
}

fn collect_tree(
    dir: &Path,
    max: usize,
    count: &mut usize,
    truncated: &mut bool,
) -> AppResult<Vec<ListEntry>> {
    let mut rows: Vec<(String, PathBuf, bool)> = Vec::new();

    for entry in std::fs::read_dir(dir).map_err(|e| {
        AppError::Other(format!("{TOOL_NAME}: read_dir {:?}: {e}", dir))
    })? {
        let entry = entry.map_err(|e| AppError::Other(format!("{TOOL_NAME}: entry: {e}")))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|e| AppError::Other(format!("{TOOL_NAME}: file_type: {e}")))?;
        rows.push((name, entry.path(), file_type.is_dir()));
    }

    rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));

    let mut out = Vec::with_capacity(rows.len());
    for (name, path, is_dir) in rows {
        if *count >= max {
            *truncated = true;
            break;
        }
        *count += 1;

        if is_dir {
            let children = if *count >= max {
                *truncated = true;
                Vec::new()
            } else {
                collect_tree(&path, max, count, truncated)?
            };
            out.push(ListEntry {
                name,
                kind: "directory",
                children: Some(children),
            });
        } else {
            out.push(ListEntry {
                name,
                kind: "file",
                children: None,
            });
        }
    }

    Ok(out)
}
