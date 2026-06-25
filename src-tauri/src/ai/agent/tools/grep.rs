//! `Grep` — literal full-text search that reports paragraph positions.
//!
//! Prose files in this app are paragraph-numbered (`[P001]`, `[P002]`, …,
//! one line = one paragraph; see [`super::paragraph`]). This tool scans a
//! file (or every text file under a directory) for an arbitrary literal
//! substring — e.g. `--` — and returns *which* paragraphs contain it, so the
//! model can jump straight to those positions with a ranged `Read` / `Edit`.
//!
//! It is deliberately a *literal* search (no regex) to stay dependency-free
//! and predictable: punctuation like `--`, `**`, `[P` is matched verbatim.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::ai::agent::tools::paragraph::split_paragraphs;
use crate::ai::agent::tools::text_decode::decode_file_bytes;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "Grep";
const DEFAULT_MAX_MATCHES: usize = 200;
const MAX_MATCHES_CAP: usize = 2_000;
const MAX_FILES_CAP: usize = 2_000;
/// Trim each reported paragraph to this many characters so a single huge
/// line can't blow up the tool result.
const SNIPPET_MAX_CHARS: usize = 240;

/// File extensions treated as searchable text when scanning a directory.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "markdown", "json", "toml", "yaml", "yml", "csv", "log", "html", "htm", "xml",
    "rs", "ts", "tsx", "js", "jsx", "css", "py",
];

#[derive(Clone)]
pub struct GrepTool {
    spec: ToolSpec,
}

impl Default for GrepTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GrepTool {
    pub fn new() -> Self {
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Search a file (or every text file under a directory) for a literal \
                    substring and report WHERE it occurs as paragraph positions. One line = one \
                    paragraph, so results are returned as `[P001]`, `[P002]`, … matching the \
                    numbering from `Read`. Use this to locate arbitrary content — punctuation \
                    included, e.g. searching `--` lists every paragraph containing `--`. This is a \
                    plain literal search (NOT a regex). After finding a position, use a ranged \
                    `Read`/`Edit` on that paragraph."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute path to the file or directory to search."
                        },
                        "query": {
                            "type": "string",
                            "description": "Literal text to find (matched verbatim, not a regex). May contain punctuation, e.g. `--`."
                        },
                        "case_sensitive": {
                            "type": "boolean",
                            "default": false,
                            "description": "When true, match case exactly. Defaults to case-insensitive."
                        },
                        "recursive": {
                            "type": "boolean",
                            "default": false,
                            "description": "When `path` is a directory, recurse into subdirectories. Ignored for a single file."
                        },
                        "max_matches": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": MAX_MATCHES_CAP,
                            "default": DEFAULT_MAX_MATCHES,
                            "description": "Stop after this many matching paragraphs (safety cap)."
                        }
                    },
                    "required": ["path", "query"]
                }),
                read_only: true,
                concurrency_safe: true,
            },
        }
    }
}

impl Tool for GrepTool {
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
        let query = input
            .get("query")
            .and_then(Value::as_str)
            .ok_or_else(|| AppError::Invalid(format!("{TOOL_NAME}: `query` must be a string")))?;
        if query.is_empty() {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `query` must be non-empty"
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

            let query = invocation
                .input
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if query.is_empty() {
                return Ok(ToolResult::error(format!("{TOOL_NAME}: `query` is empty")));
            }

            let case_sensitive = invocation
                .input
                .get("case_sensitive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let recursive = invocation
                .input
                .get("recursive")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let max_matches = invocation
                .input
                .get("max_matches")
                .and_then(Value::as_u64)
                .map(|n| n as usize)
                .unwrap_or(DEFAULT_MAX_MATCHES)
                .clamp(1, MAX_MATCHES_CAP);

            let canonical = std::fs::canonicalize(&path).map_err(|e| {
                AppError::Other(format!("{TOOL_NAME}: canonicalize {:?}: {e}", path))
            })?;

            let needle = if case_sensitive {
                query.clone()
            } else {
                query.to_lowercase()
            };

            // Collect the file(s) to scan.
            let mut files: Vec<PathBuf> = Vec::new();
            let mut files_capped = false;
            if canonical.is_dir() {
                collect_text_files(&canonical, recursive, &mut files, &mut files_capped);
            } else {
                files.push(canonical.clone());
            }

            let mut file_results: Vec<Value> = Vec::new();
            let mut total_matches = 0usize;
            let mut matches_capped = false;

            'outer: for file in &files {
                let Ok(bytes) = std::fs::read(file) else {
                    continue;
                };
                let text = decode_file_bytes(&bytes);
                let mut matches: Vec<Value> = Vec::new();

                for (i, line) in split_paragraphs(&text).into_iter().enumerate() {
                    let hay = if case_sensitive {
                        line.clone()
                    } else {
                        line.to_lowercase()
                    };
                    let occurrences = count_occurrences(&hay, &needle);
                    if occurrences == 0 {
                        continue;
                    }
                    matches.push(json!({
                        "paragraph": i + 1,
                        "label": format!("[P{:03}]", i + 1),
                        "occurrences": occurrences,
                        "text": snippet(&line),
                    }));
                    total_matches += 1;
                    if total_matches >= max_matches {
                        matches_capped = true;
                    }
                    if matches_capped {
                        if !matches.is_empty() {
                            file_results.push(json!({
                                "path": file.to_string_lossy(),
                                "matches": matches,
                            }));
                        }
                        break 'outer;
                    }
                }

                if !matches.is_empty() {
                    file_results.push(json!({
                        "path": file.to_string_lossy(),
                        "matches": matches,
                    }));
                }
            }

            // For the single-file case, flatten the matches to the top level
            // so the model sees paragraph positions directly.
            if !canonical.is_dir() {
                let matches = file_results
                    .into_iter()
                    .next()
                    .and_then(|f| f.get("matches").cloned())
                    .unwrap_or_else(|| json!([]));
                return Ok(ToolResult::ok(json!({
                    "path": canonical.to_string_lossy(),
                    "query": query,
                    "case_sensitive": case_sensitive,
                    "total_matches": total_matches,
                    "truncated": matches_capped,
                    "matches": matches,
                })));
            }

            Ok(ToolResult::ok(json!({
                "path": canonical.to_string_lossy(),
                "query": query,
                "case_sensitive": case_sensitive,
                "recursive": recursive,
                "files_searched": files.len(),
                "files_capped": files_capped,
                "total_matches": total_matches,
                "truncated": matches_capped,
                "results": file_results,
            })))
        })
    }
}

/// Count non-overlapping occurrences of `needle` in `hay`.
fn count_occurrences(hay: &str, needle: &str) -> usize {
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut start = 0;
    while let Some(pos) = hay[start..].find(needle) {
        count += 1;
        start += pos + needle.len();
    }
    count
}

/// Trim a paragraph to [`SNIPPET_MAX_CHARS`] characters (char-safe).
fn snippet(s: &str) -> String {
    if s.chars().count() <= SNIPPET_MAX_CHARS {
        return s.to_string();
    }
    let truncated: String = s.chars().take(SNIPPET_MAX_CHARS).collect();
    format!("{truncated}…")
}

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| TEXT_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn collect_text_files(dir: &Path, recursive: bool, out: &mut Vec<PathBuf>, capped: &mut bool) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES_CAP {
            *capped = true;
            return;
        }
        let path = entry.path();
        if path.is_dir() {
            if recursive {
                collect_text_files(&path, recursive, out, capped);
                if *capped {
                    return;
                }
            }
        } else if is_text_file(&path) {
            out.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_basic() {
        assert_eq!(count_occurrences("a--b--c", "--"), 2);
        assert_eq!(count_occurrences("aaaa", "aa"), 2);
        assert_eq!(count_occurrences("none here", "xyz"), 0);
        assert_eq!(count_occurrences("anything", ""), 0);
    }

    #[test]
    fn snippet_truncates_by_char() {
        let s: String = "中".repeat(SNIPPET_MAX_CHARS + 10);
        let out = snippet(&s);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), SNIPPET_MAX_CHARS + 1);
    }

    #[test]
    fn snippet_keeps_short() {
        assert_eq!(snippet("hello"), "hello");
    }
}
