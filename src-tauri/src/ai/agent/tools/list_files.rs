//! `ListFiles` — enumerate files and subdirectories under a path.
//!
//! Always returns a fully nested tree: every directory node includes a `children`
//! array (possibly empty) with its files and subfolders inside.

use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::ai::agent::core::context::AbortSignal;
use crate::ai::agent::tools::paragraph::paragraph_count;
use crate::ai::agent::tools::project_path::{self, display_path, DIR_REF_DESC};
use crate::ai::agent::tools::text_decode::decode_file_bytes;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "ListFiles";
const DEFAULT_MAX_ENTRIES: usize = 500;
const MAX_ENTRIES_CAP: usize = 5_000;
/// Largest file whose paragraphs are counted for the listing.
const MAX_COUNT_BYTES: u64 = 8 * 1024 * 1024;

/// File extensions treated as paragraph-countable text (same set as `Grep`).
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "json", "toml", "yaml", "yml", "csv", "log", "html", "htm", "xml",
    "rs", "ts", "tsx", "js", "jsx", "css", "py",
];

#[derive(Clone, Serialize)]
struct ListEntry {
    name: String,
    kind: &'static str,
    /// Present on every `directory` node (may be `[]`). Omitted on `file` nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    children: Option<Vec<ListEntry>>,
    /// Present on text `file` nodes: one line = one paragraph (see [`super::paragraph`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    paragraphs: Option<usize>,
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
                    Returns `{ success, entries: [{ name, kind, children?, paragraphs? }] }` where \
                    each directory has `children: [...]` containing its files and \
                    subfolders (recursively). `kind` is `directory` or `file`. \
                    Text files also include `paragraphs` (one line = one paragraph, \
                    matching `Read`/`Edit` numbering). \
                    Use instead of Bash `dir`/`ls` for reliable Unicode paths."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": DIR_REF_DESC
                        },
                        "max_entries": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": MAX_ENTRIES_CAP,
                            "default": DEFAULT_MAX_ENTRIES,
                            "description": "Stop after this many nodes total (safety cap for huge trees)."
                        }
                    },
                    "required": []
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
        if let Some(path) = input.get("path") {
            if !path.is_string() && !path.is_null() {
                return Err(AppError::Invalid(format!(
                    "{TOOL_NAME}: `path` must be a string"
                )));
            }
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let raw = invocation.input.get("path").and_then(Value::as_str);
            let path = project_path::resolve_project_dir(
                &invocation.context.cwd,
                raw,
                TOOL_NAME,
            )?;

            let max_entries = invocation
                .input
                .get("max_entries")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_ENTRIES)
                .clamp(1, MAX_ENTRIES_CAP);

            let canonical = path;
            if !canonical.is_dir() {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: not a directory: {}",
                    canonical.display()
                )));
            }

            // Walking a tree is blocking IO of unknown size; keep it off the
            // async worker threads and let a cancelled turn stop it.
            let abort = invocation.context.abort.clone();
            let root = canonical.clone();
            let walk = tokio::task::spawn_blocking(move || {
                let mut state = WalkState {
                    root: root.clone(),
                    count: 0,
                    truncated: false,
                    max: max_entries,
                    abort,
                };
                let entries = collect_tree(&root, 0, &mut state)?;
                Ok::<_, AppError>((entries, state.truncated, state.abort.aborted()))
            })
            .await
            .map_err(|e| AppError::Other(format!("{TOOL_NAME}: walk task failed: {e}")))?;
            let (entries, truncated, cancelled) = walk?;

            if cancelled {
                return Ok(ToolResult::error(format!("{TOOL_NAME}: listing cancelled")));
            }

            Ok(ToolResult::ok(json!({
                "success": true,
                "path": display_path(&canonical),
                "truncated": truncated,
                "entries": entries,
            })))
        })
    }
}

struct WalkState {
    root: PathBuf,
    count: usize,
    truncated: bool,
    max: usize,
    abort: AbortSignal,
}

/// Deepest nesting the tree walk will report.
const MAX_TREE_DEPTH: usize = 24;

/// Build the nested listing.
///
/// Symlinks and Windows junctions are listed as-is but never descended into: a
/// link back to an ancestor would otherwise recurse until the stack overflows,
/// and a link out of the project would expose paths the caller never asked for.
fn collect_tree(dir: &Path, depth: usize, state: &mut WalkState) -> AppResult<Vec<ListEntry>> {
    if depth > MAX_TREE_DEPTH {
        state.truncated = true;
        return Ok(Vec::new());
    }
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
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if !project_path::is_within(&state.root, &path) {
            continue;
        }
        let descendable = file_type.is_dir() && !is_reparse_point(&path);
        if !crate::ai::agent::tools::pe_docs::include_listed_path(&path, descendable) {
            continue;
        }
        rows.push((name, path, descendable));
    }

    rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.to_lowercase().cmp(&b.0.to_lowercase())));

    let mut out = Vec::with_capacity(rows.len());
    for (name, path, is_dir) in rows {
        if state.abort.aborted() {
            return Ok(out);
        }
        if state.count >= state.max {
            state.truncated = true;
            break;
        }
        state.count += 1;

        if is_dir {
            let children = if state.count >= state.max {
                state.truncated = true;
                Vec::new()
            } else {
                collect_tree(&path, depth + 1, state)?
            };
            out.push(ListEntry {
                name,
                kind: "directory",
                children: Some(children),
                paragraphs: None,
            });
        } else {
            out.push(ListEntry {
                name,
                kind: "file",
                children: None,
                paragraphs: file_paragraph_count(&path),
            });
        }
    }

    Ok(out)
}

/// True when `path` is a Windows reparse point (junction / directory symlink),
/// which `file_type().is_dir()` reports as an ordinary directory.
#[cfg(windows)]
fn is_reparse_point(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    std::fs::symlink_metadata(path)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(true)
}

#[cfg(not(windows))]
fn is_reparse_point(_path: &Path) -> bool {
    false
}

fn is_text_file(path: &Path) -> bool {
    if crate::ai::agent::tools::pe_docs::pe_docs_only() {
        return crate::ai::agent::tools::pe_docs::is_doc_path(path);
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| TEXT_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Paragraph count for a listing row.
///
/// Reads at most [`MAX_COUNT_BYTES`]: the listing is a directory overview, and
/// slurping every text file in a large project in full to count its lines is
/// how a harmless `ListFiles` becomes a memory spike.
fn file_paragraph_count(path: &Path) -> Option<usize> {
    use std::io::Read;

    if !is_text_file(path) {
        return None;
    }
    let file = std::fs::File::open(path).ok()?;
    if file.metadata().ok()?.len() > MAX_COUNT_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_COUNT_BYTES).read_to_end(&mut bytes).ok()?;
    Some(paragraph_count(&decode_file_bytes(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_count_for_text_file() {
        assert_eq!(paragraph_count("a\n\nb"), 3);
        assert_eq!(paragraph_count("a\nb"), 2);
    }

    #[test]
    fn is_text_file_by_extension() {
        assert!(is_text_file(Path::new("/x/story.txt")));
        assert!(is_text_file(Path::new("/x/readme.MD")));
        assert!(!is_text_file(Path::new("/x/image.png")));
        assert!(!is_text_file(Path::new("/x/noext")));
    }
}
