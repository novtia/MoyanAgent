//! `ListFiles` — enumerate files and subdirectories under a path.
//!
//! Gives the model a structured directory listing without shelling out to
//! `dir` / `ls`, so Unicode paths and names are handled correctly on Windows.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "ListFiles";
const DEFAULT_MAX_ENTRIES: usize = 500;
const MAX_ENTRIES_CAP: usize = 5_000;

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
                description: "List files and subdirectories under a directory. \
                    Returns `{ success: true, files: [names…] }`. Use this instead of \
                    Bash `dir`/`ls` when you need a reliable listing (especially with \
                    non-ASCII paths on Windows)."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the directory to list."
                        },
                        "recursive": {
                            "type": "boolean",
                            "default": false,
                            "description": "When true, include all nested entries (depth-first). \
                                Names are relative paths from the listed directory."
                        },
                        "max_entries": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": MAX_ENTRIES_CAP,
                            "default": DEFAULT_MAX_ENTRIES,
                            "description": "Stop after this many entries (safety cap for huge trees)."
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

            let recursive = invocation
                .input
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
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

            let mut items = Vec::new();
            let mut capped = false;
            if recursive {
                collect_recursive(&canonical, &canonical, &mut items, max_entries, &mut capped)?;
            } else {
                collect_shallow(&canonical, &canonical, &mut items)?;
            }

            items.sort_by(|a, b| {
                b.is_dir
                    .cmp(&a.is_dir)
                    .then_with(|| a.name.cmp(&b.name))
            });

            let files: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();

            Ok(ToolResult::ok(json!({
                "success": true,
                "files": files,
            })))
        })
    }
}

struct ListedItem {
    name: String,
    is_dir: bool,
}

fn collect_shallow(root: &Path, dir: &Path, out: &mut Vec<ListedItem>) -> AppResult<()> {
    for entry in std::fs::read_dir(dir).map_err(|e| {
        AppError::Other(format!("ListFiles: read_dir {:?}: {e}", dir))
    })? {
        let entry = entry.map_err(|e| AppError::Other(format!("ListFiles: entry: {e}")))?;
        if let Some(item) = entry_to_item(root, &entry)? {
            out.push(item);
        }
    }
    Ok(())
}

fn collect_recursive(
    root: &Path,
    dir: &Path,
    out: &mut Vec<ListedItem>,
    max: usize,
    capped: &mut bool,
) -> AppResult<()> {
    if out.len() >= max {
        *capped = true;
        return Ok(());
    }
    collect_shallow(root, dir, out)?;
    if out.len() >= max {
        *capped = true;
        out.truncate(max);
        return Ok(());
    }

    let subdirs: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| AppError::Other(format!("ListFiles: read_dir {:?}: {e}", dir)))?
        .filter_map(|e| e.ok())
        .filter_map(|e| e.path().is_dir().then_some(e.path()))
        .collect();

    for sub in subdirs {
        if out.len() >= max {
            *capped = true;
            return Ok(());
        }
        collect_recursive(root, &sub, out, max, capped)?;
    }
    Ok(())
}

fn entry_to_item(root: &Path, entry: &std::fs::DirEntry) -> AppResult<Option<ListedItem>> {
    let path = entry.path();
    let file_type = entry
        .file_type()
        .map_err(|e| AppError::Other(format!("ListFiles: file_type: {e}")))?;

    let name = path
        .strip_prefix(root)
        .ok()
        .and_then(|p| {
            let s = p.to_string_lossy().replace('\\', "/");
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        })
        .unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());

    if name.is_empty() {
        return Ok(None);
    }

    Ok(Some(ListedItem {
        name,
        is_dir: file_type.is_dir(),
    }))
}
