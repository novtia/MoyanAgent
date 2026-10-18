//! `Grep` — literal full-text search that reports paragraph positions.
//!
//! Prose files in this app are paragraph-numbered (`[P001]`, `[P002]`, …,
//! one line = one paragraph; see [`super::paragraph`]). This tool scans one or
//! more files / directories for one or more literal substrings — e.g. `--` —
//! and returns *which* paragraphs contain them, so the model can jump straight
//! to those positions with a ranged `Read` / `Edit`.
//!
//! `query` and `path` are lists, so several terms and several targets are
//! searched in a single call: the files are read and decoded once and every query is matched
//! against each paragraph, so "find A, B and C across these chapters" costs one
//! tool round trip instead of one per term.
//!
//! It is deliberately a *literal* search (no regex) to stay dependency-free
//! and predictable: punctuation like `--`, `**`, `[P` is matched verbatim.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::ai::agent::core::context::AbortSignal;
use crate::ai::agent::tools::paragraph::split_paragraphs;
use crate::ai::agent::tools::project_path::{self, display_path, FILE_REF_DESC};
use crate::ai::agent::tools::text_decode::decode_file_bytes;
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "Grep";
const DEFAULT_MAX_MATCHES: usize = 200;
const MAX_MATCHES_CAP: usize = 2_000;
const MAX_FILES_CAP: usize = 2_000;
/// Most queries / targets accepted in one call.
const MAX_QUERIES: usize = 20;
const MAX_PATHS: usize = 20;
/// Trim each reported paragraph to this many characters so a single huge
/// line can't blow up the tool result.
const SNIPPET_MAX_CHARS: usize = 240;
/// Deepest directory nesting the recursive scan will walk.
const MAX_SCAN_DEPTH: usize = 24;
/// Only this much of any one file is scanned. Without a bound, a single
/// multi-gigabyte log in the project turns a search into an out-of-memory kill.
const MAX_FILE_SCAN_BYTES: u64 = 8 * 1024 * 1024;

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
        let path_desc = format!("{FILE_REF_DESC} For a directory, pass a folder breadcrumb only.");
        Self {
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Search files for literal substrings and report WHERE they occur as \
                    paragraph positions. One line = one paragraph, so results are returned as \
                    `[P001]`, `[P002]`, … matching the numbering from `Read`. \
                    `path` and `query` are both lists: put EVERY file/folder and EVERY term you \
                    want in this one call (one item is fine too) instead of issuing one Grep per \
                    term or per file. Each paragraph that contains any of the queries is listed \
                    once, with `matched_queries` saying which ones hit, and `per_query` \
                    summarises how many paragraphs each query matched (0 = not found). \
                    Punctuation is fine, e.g. searching `--` lists every paragraph containing \
                    `--`. This is a plain literal search (NOT a regex). After finding positions, \
                    use a ranged `Read` / `Edit` on those paragraphs."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": MAX_PATHS,
                            "items": { "type": "string" },
                            "description": format!("Files and/or folders to search; one or several. {path_desc}")
                        },
                        "query": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": MAX_QUERIES,
                            "items": { "type": "string" },
                            "description": "Literal texts to find; one or several (each matched verbatim, not a regex; may contain punctuation such as `--`). A paragraph matches if it contains ANY of them."
                        },
                        "case_sensitive": {
                            "type": "boolean",
                            "default": false,
                            "description": "When true, match case exactly. Defaults to case-insensitive."
                        },
                        "recursive": {
                            "type": "boolean",
                            "default": false,
                            "description": "For directory targets, recurse into subdirectories. Ignored for single files."
                        },
                        "max_matches": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": MAX_MATCHES_CAP,
                            "default": DEFAULT_MAX_MATCHES,
                            "description": "Stop after this many matching paragraphs in total (safety cap)."
                        }
                    }
                }),
                read_only: true,
                concurrency_safe: true,
            },
        }
    }
}

/// Collect the strings under `key`, which holds an array of strings (a bare
/// string is accepted too, as a one-item list). Empty entries are dropped and
/// duplicates removed while the caller's order is preserved. `trim` is for
/// paths; queries stay verbatim (a query of `" -- "` is meaningful).
fn string_list(input: &Value, key: &str, trim: bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut push = |s: &str| {
        let s = if trim { s.trim() } else { s };
        if s.trim().is_empty() {
            return;
        }
        if seen.insert(s.to_string()) {
            out.push(s.to_string());
        }
    };
    match input.get(key) {
        Some(Value::String(s)) => push(s),
        Some(Value::Array(items)) => {
            for item in items {
                if let Some(s) = item.as_str() {
                    push(s);
                }
            }
        }
        _ => {}
    }
    out
}

fn requested_queries(input: &Value) -> Vec<String> {
    string_list(input, "query", false)
}

fn requested_paths(input: &Value) -> Vec<String> {
    string_list(input, "path", true)
}

impl Tool for GrepTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let paths = requested_paths(input);
        if paths.is_empty() {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `path` must list at least one file or folder"
            )));
        }
        if paths.len() > MAX_PATHS {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: at most {MAX_PATHS} paths per call (got {})",
                paths.len()
            )));
        }
        let queries = requested_queries(input);
        if queries.is_empty() {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `query` must list at least one non-empty string"
            )));
        }
        if queries.len() > MAX_QUERIES {
            return Err(AppError::Invalid(format!(
                "{TOOL_NAME}: at most {MAX_QUERIES} queries per call (got {})",
                queries.len()
            )));
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let raw_paths = requested_paths(&invocation.input);
            let queries = requested_queries(&invocation.input);
            if raw_paths.is_empty() {
                return Ok(ToolResult::error(format!("{TOOL_NAME}: `path` is empty")));
            }
            if queries.is_empty() {
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

            // Resolve every target. With a single path a bad reference is a
            // hard error (as before); with several, the good ones still run and
            // the bad ones are reported so one typo doesn't cost the batch.
            let mut roots: Vec<PathBuf> = Vec::new();
            let mut path_errors: Vec<Value> = Vec::new();
            let mut first_error: Option<AppError> = None;
            for raw in &raw_paths {
                let resolved = (|| -> AppResult<PathBuf> {
                    let path = project_path::resolve_project_file_or_dir(
                        &invocation.context.cwd,
                        raw,
                        TOOL_NAME,
                    )?;
                    let canonical = std::fs::canonicalize(&path).map_err(|e| {
                        AppError::Other(format!("{TOOL_NAME}: canonicalize {:?}: {e}", path))
                    })?;
                    crate::ai::agent::tools::pe_docs::refuse_nondoc(TOOL_NAME, &canonical)?;
                    Ok(canonical)
                })();
                match resolved {
                    Ok(canonical) => {
                        if !roots.contains(&canonical) {
                            roots.push(canonical);
                        }
                    }
                    Err(e) => {
                        path_errors.push(json!({ "path": raw, "error": e.to_string() }));
                        first_error.get_or_insert(e);
                    }
                }
            }
            if roots.is_empty() {
                return match first_error {
                    Some(e) if raw_paths.len() == 1 => Err(e),
                    Some(e) => Ok(ToolResult::error(format!(
                        "{TOOL_NAME}: none of the paths could be searched: {e}"
                    ))),
                    None => Ok(ToolResult::error(format!("{TOOL_NAME}: no paths to search"))),
                };
            }

            let single_file = roots.len() == 1 && !roots[0].is_dir();
            let multi_query = queries.len() > 1;

            // Scanning a directory tree is unbounded blocking IO. On the async
            // runtime it would occupy a worker thread for the whole search and
            // ignore cancellation until the last file; `spawn_blocking` plus an
            // abort check per file makes it interruptible and keeps the runtime
            // responsive.
            let abort = invocation.context.abort.clone();
            let scan_roots = roots.clone();
            let scan_queries = queries.clone();
            let scan = tokio::task::spawn_blocking(move || {
                run_scan(ScanRequest {
                    roots: scan_roots,
                    queries: scan_queries,
                    case_sensitive,
                    recursive,
                    max_matches,
                    abort,
                })
            })
            .await
            .map_err(|e| AppError::Other(format!("{TOOL_NAME}: scan task failed: {e}")))?;

            if scan.cancelled {
                return Ok(ToolResult::error(format!("{TOOL_NAME}: search cancelled")));
            }

            let per_query: Vec<Value> = queries
                .iter()
                .enumerate()
                .map(|(i, q)| {
                    json!({
                        "query": q,
                        "paragraphs": scan.per_query_paragraphs[i],
                        "occurrences": scan.per_query_occurrences[i],
                    })
                })
                .collect();
            // Display form used by the UI card: `a | b | c`.
            let query_label = queries.join(" | ");

            // Single file + single query: flatten the matches to the top level
            // so the model sees paragraph positions directly.
            if single_file && !multi_query {
                let matches = scan
                    .file_results
                    .into_iter()
                    .next()
                    .and_then(|f| f.get("matches").cloned())
                    .unwrap_or_else(|| json!([]));
                let mut body = json!({
                    "path": display_path(&roots[0]),
                    "query": queries[0],
                    "case_sensitive": case_sensitive,
                    "total_matches": scan.total_matches,
                    "truncated": scan.matches_capped,
                    "matches": matches,
                });
                if scan.files_truncated > 0 {
                    body["file_truncated"] = json!(true);
                }
                if !path_errors.is_empty() {
                    body["path_errors"] = json!(path_errors);
                }
                return Ok(ToolResult::ok(body));
            }

            let mut body = json!({
                "query": query_label,
                "queries": queries,
                "paths": roots.iter().map(|r| display_path(r)).collect::<Vec<_>>(),
                "case_sensitive": case_sensitive,
                "recursive": recursive,
                "files_searched": scan.files_searched,
                "files_capped": scan.files_capped,
                "total_matches": scan.total_matches,
                "truncated": scan.matches_capped,
                "results": scan.file_results,
            });
            if roots.len() == 1 {
                body["path"] = json!(display_path(&roots[0]));
            }
            if multi_query {
                body["per_query"] = json!(per_query);
            }
            if scan.files_truncated > 0 {
                body["files_truncated"] = json!(scan.files_truncated);
            }
            if !path_errors.is_empty() {
                body["path_errors"] = json!(path_errors);
            }
            Ok(ToolResult::ok(body))
        })
    }
}

struct ScanRequest {
    roots: Vec<PathBuf>,
    queries: Vec<String>,
    case_sensitive: bool,
    recursive: bool,
    max_matches: usize,
    abort: AbortSignal,
}

#[derive(Default)]
struct ScanOutcome {
    file_results: Vec<Value>,
    files_searched: usize,
    total_matches: usize,
    matches_capped: bool,
    files_capped: bool,
    /// Files whose tail was not scanned because of [`MAX_FILE_SCAN_BYTES`].
    files_truncated: usize,
    cancelled: bool,
    /// Per query: paragraphs containing it / total occurrences.
    per_query_paragraphs: Vec<usize>,
    per_query_occurrences: Vec<usize>,
}

fn run_scan(req: ScanRequest) -> ScanOutcome {
    let mut out = ScanOutcome {
        per_query_paragraphs: vec![0; req.queries.len()],
        per_query_occurrences: vec![0; req.queries.len()],
        ..ScanOutcome::default()
    };
    let multi_query = req.queries.len() > 1;
    let needles: Vec<String> = req
        .queries
        .iter()
        .map(|q| {
            if req.case_sensitive {
                q.clone()
            } else {
                q.to_lowercase()
            }
        })
        .collect();

    let mut files: Vec<PathBuf> = Vec::new();
    for root in &req.roots {
        if root.is_dir() {
            collect_text_files(
                root,
                root,
                req.recursive,
                0,
                &mut files,
                &mut out.files_capped,
            );
        } else {
            files.push(root.clone());
        }
    }
    // Overlapping targets (a folder plus a file inside it) must not be scanned,
    // and so reported, twice.
    let mut seen: HashSet<PathBuf> = HashSet::new();
    files.retain(|f| seen.insert(f.clone()));
    out.files_searched = files.len();

    'outer: for file in &files {
        if req.abort.aborted() {
            out.cancelled = true;
            return out;
        }
        let Some((bytes, truncated)) = read_prefix(file) else {
            continue;
        };
        if truncated {
            out.files_truncated += 1;
        }
        let text = decode_file_bytes(&bytes);
        let mut matches: Vec<Value> = Vec::new();

        for (i, line) in split_paragraphs(&text).into_iter().enumerate() {
            let hay = if req.case_sensitive {
                line.clone()
            } else {
                line.to_lowercase()
            };
            let mut occurrences = 0usize;
            let mut matched: Vec<&str> = Vec::new();
            for (qi, needle) in needles.iter().enumerate() {
                let n = count_occurrences(&hay, needle);
                if n == 0 {
                    continue;
                }
                occurrences += n;
                out.per_query_paragraphs[qi] += 1;
                out.per_query_occurrences[qi] += n;
                matched.push(req.queries[qi].as_str());
            }
            if occurrences == 0 {
                continue;
            }
            let mut m = json!({
                "paragraph": i + 1,
                "label": format!("[P{:03}]", i + 1),
                "occurrences": occurrences,
                "text": snippet(&line),
            });
            if multi_query {
                m["matched_queries"] = json!(matched);
            }
            matches.push(m);
            out.total_matches += 1;
            if out.total_matches >= req.max_matches {
                out.matches_capped = true;
                out.file_results.push(json!({
                    "path": display_path(file),
                    "matches": matches,
                }));
                break 'outer;
            }
        }

        if !matches.is_empty() {
            out.file_results.push(json!({
                "path": display_path(file),
                "matches": matches,
            }));
        }
    }

    out
}

/// Read at most [`MAX_FILE_SCAN_BYTES`] of `path`, reporting whether the file
/// continued past the cap.
fn read_prefix(path: &Path) -> Option<(Vec<u8>, bool)> {
    use std::io::Read;

    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut buf = Vec::with_capacity(len.min(MAX_FILE_SCAN_BYTES) as usize);
    file.take(MAX_FILE_SCAN_BYTES).read_to_end(&mut buf).ok()?;
    Some((buf, len > MAX_FILE_SCAN_BYTES))
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
    if crate::ai::agent::tools::pe_docs::pe_docs_only() {
        return crate::ai::agent::tools::pe_docs::is_doc_path(path);
    }
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| TEXT_EXTENSIONS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

/// Gather the text files to scan.
///
/// Symlinks and Windows junctions are skipped rather than followed: one link
/// pointing at a parent directory makes the walk recurse forever, and one
/// pointing outside the project makes a project-scoped search report files the
/// caller never granted access to.
fn collect_text_files(
    dir: &Path,
    root: &Path,
    recursive: bool,
    depth: usize,
    out: &mut Vec<PathBuf>,
    capped: &mut bool,
) {
    if depth > MAX_SCAN_DEPTH {
        *capped = true;
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES_CAP {
            *capped = true;
            return;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            if recursive && !is_reparse_point(&path) {
                collect_text_files(&path, root, recursive, depth + 1, out, capped);
                // Only a full result set ends the walk. A subtree skipped for
                // being too deep still leaves this directory's own files, and
                // the siblings after it, worth collecting.
                if out.len() >= MAX_FILES_CAP {
                    *capped = true;
                    return;
                }
            }
        } else if file_type.is_file() && is_text_file(&path) && project_path::is_within(root, &path)
        {
            out.push(path);
        }
    }
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

    /// A deep tree must stop the walk instead of the stack.
    #[test]
    fn collection_stops_at_the_depth_limit() {
        let root = std::env::temp_dir().join(format!("moyan-grep-depth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut deep = root.clone();
        for level in 0..(MAX_SCAN_DEPTH + 5) {
            deep = deep.join(format!("l{level}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("buried.txt"), "needle").unwrap();
        std::fs::write(root.join("top.txt"), "needle").unwrap();

        let mut files = Vec::new();
        let mut capped = false;
        collect_text_files(&root, &root, true, 0, &mut files, &mut capped);

        assert!(capped, "the walk reports that it stopped early");
        assert!(
            files.iter().any(|p| p.ends_with("top.txt")),
            "shallow files are still found"
        );
        assert!(
            !files.iter().any(|p| p.ends_with("buried.txt")),
            "the walk must not descend past the limit"
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_huge_file_is_scanned_only_up_to_the_cap() {
        let dir = std::env::temp_dir().join(format!("moyan-grep-cap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let big = dir.join("big.txt");
        let filler = vec![b'a'; (MAX_FILE_SCAN_BYTES + 1024) as usize];
        std::fs::write(&big, filler).unwrap();

        let (bytes, truncated) = read_prefix(&big).expect("readable");
        assert_eq!(bytes.len() as u64, MAX_FILE_SCAN_BYTES);
        assert!(truncated, "the caller must learn the tail was skipped");

        let small = dir.join("small.txt");
        std::fs::write(&small, "hello").unwrap();
        let (bytes, truncated) = read_prefix(&small).expect("readable");
        assert_eq!(bytes, b"hello");
        assert!(!truncated);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn query_and_path_lists_accept_strings_and_arrays() {
        let input = json!({
            "query": ["b", "a", "b", "", "  "],
            "path": [" y.md ", "x.md"],
        });
        assert_eq!(requested_queries(&input), vec!["b", "a"]);
        assert_eq!(requested_paths(&input), vec!["y.md", "x.md"]);

        // A bare string is the one-item case.
        let bare = json!({ "query": "a", "path": " x.md " });
        assert_eq!(requested_queries(&bare), vec!["a"]);
        assert_eq!(requested_paths(&bare), vec!["x.md"]);

        let tool = GrepTool::new();
        assert!(tool.validate(&input).is_ok());
        assert!(tool.validate(&bare).is_ok());
        assert!(tool.validate(&json!({ "path": "x.md" })).is_err());
        assert!(tool.validate(&json!({ "query": "a" })).is_err());
    }

    #[tokio::test]
    async fn several_queries_across_several_files_run_in_one_call() {
        use crate::ai::agent::core::context::ToolUseContextBuilder;
        use crate::ai::agent::types::{AgentId, MessageId};

        let dir = std::env::temp_dir().join(format!("moyan-grep-multi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "alpha one\nbeta two\nalpha beta\nnothing").unwrap();
        std::fs::write(dir.join("b.txt"), "gamma\nBETA again").unwrap();

        let ctx = ToolUseContextBuilder::new(AgentId::new(), dir.clone())
            .build()
            .0;
        let res = GrepTool::new()
            .execute(ToolInvocation {
                id: MessageId("grep".into()),
                input: json!({
                    "query": ["alpha", "beta", "missing"],
                    "path": ["a.txt", "b.txt"],
                }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(!res.is_error, "unexpected error: {:?}", res.content);

        // a.txt: P1 alpha, P2 beta, P3 alpha+beta; b.txt: P2 beta (case-insensitive).
        assert_eq!(res.content["total_matches"], 4);
        let results = res.content["results"].as_array().unwrap();
        assert_eq!(results.len(), 2);
        let both = &results[0]["matches"][2];
        assert_eq!(both["paragraph"], 3);
        assert_eq!(both["occurrences"], 2);
        assert_eq!(both["matched_queries"], json!(["alpha", "beta"]));

        let per_query = res.content["per_query"].as_array().unwrap();
        assert_eq!(per_query[0]["paragraphs"], 2);
        assert_eq!(per_query[1]["paragraphs"], 3);
        assert_eq!(per_query[2]["paragraphs"], 0, "a miss is reported, not hidden");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn one_bad_path_does_not_sink_the_batch() {
        use crate::ai::agent::core::context::ToolUseContextBuilder;
        use crate::ai::agent::types::{AgentId, MessageId};

        let dir = std::env::temp_dir().join(format!("moyan-grep-badpath-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "needle").unwrap();

        let ctx = ToolUseContextBuilder::new(AgentId::new(), dir.clone())
            .build()
            .0;
        let res = GrepTool::new()
            .execute(ToolInvocation {
                id: MessageId("grep".into()),
                input: json!({ "query": ["needle"], "path": ["a.txt", "does-not-exist.txt"] }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(!res.is_error, "unexpected error: {:?}", res.content);
        assert_eq!(res.content["total_matches"], 1);
        assert_eq!(
            res.content["path_errors"].as_array().unwrap().len(),
            1,
            "{:?}",
            res.content
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
