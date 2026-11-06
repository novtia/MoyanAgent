//! `FindReplace` — find / replace in a single file, literal or regex.
//!
//! Where `Edit` swaps one unique span, this tool is for bulk work: renaming a
//! term everywhere, regex rewrites with capture groups, capped replacement
//! counts, and a `dry_run` that lists the hits without touching the file.
//!
//! Literal matching mirrors `Edit`: verbatim first, then quote-folded
//! (ASCII `"` / `“”` / `「」` are the same glyph) and CRLF/LF swaps, so a model
//! that cannot see the file's exact quote style still lands. Regex mode matches
//! as written. Writes snapshot the pre-image, keep the file's encoding and BOM,
//! refresh the read receipt and record a pending diff like `Edit` does.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use regex::RegexBuilder;
use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{FileChangeRecord, FileOp, FileSnapshotStore};
use crate::ai::agent::tools::project_path::{self, FILE_REF_DESC};
use crate::ai::agent::tools::read_receipt::{clear_receipt, record_receipt};
use crate::ai::agent::tools::text_decode::{
    find_quote_folded_ranges, read_text_file, remap_quotes_to_sample, write_text_file,
};
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::data::db::DbPool;
use crate::data::pending_diff;
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "FindReplace";
/// Hits listed in the result; the count still covers all of them.
const MAX_LISTED_MATCHES: usize = 20;
/// Characters of surrounding text shown on each side of a listed hit.
const CONTEXT_CHARS: usize = 20;

#[derive(Clone)]
pub struct FindReplaceTool {
    spec: ToolSpec,
    snapshots: Arc<FileSnapshotStore>,
    pool: Option<Arc<DbPool>>,
}

impl FindReplaceTool {
    pub fn new(snapshots: Arc<FileSnapshotStore>) -> Self {
        Self {
            snapshots,
            pool: None,
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Find and replace text in ONE file: every occurrence by default, \
                    literal or regex. Use it for renames, unifying wording, and pattern \
                    rewrites; use Edit to rewrite one specific passage. Call with \
                    `dry_run: true` first when unsure what will match — it lists the hits \
                    (line number plus surrounding text) and writes nothing. With `regex: true`, \
                    `replace` may reference capture groups as `$1` or `${name}` (write `$$` \
                    for a literal dollar sign). Literal matching treats ASCII and typographic \
                    quotes as equivalent. An empty `replace` deletes the matches."
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
                        "find": {
                            "type": "string",
                            "description": "Text to find (a regular expression when `regex` is true). Non-empty."
                        },
                        "replace": {
                            "type": "string",
                            "description": "Replacement text. With `regex: true`, `$1` / `${name}` insert capture groups. Empty string deletes the matches."
                        },
                        "regex": {
                            "type": "boolean",
                            "description": "Treat `find` as a regular expression. Defaults to false (literal text)."
                        },
                        "case_sensitive": {
                            "type": "boolean",
                            "description": "Match letter case exactly. Defaults to true."
                        },
                        "max_replacements": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Replace only the first N matches (in file order). Omit to replace all."
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "List the matches without modifying the file. Defaults to false."
                        }
                    },
                    "required": ["path", "find", "replace"]
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
}

/// One planned replacement: byte range in the original text plus its new text.
#[derive(Debug, Clone, PartialEq)]
struct Hit {
    start: usize,
    end: usize,
    new: String,
}

#[derive(Debug)]
struct Options<'a> {
    find: &'a str,
    replace: &'a str,
    regex: bool,
    case_sensitive: bool,
}

/// All hits in file order. Regex errors surface as `Err`.
fn find_hits(text: &str, opts: &Options<'_>) -> Result<Vec<Hit>, String> {
    if opts.regex || !opts.case_sensitive {
        let pattern = if opts.regex {
            opts.find.to_string()
        } else {
            regex::escape(opts.find)
        };
        let re = RegexBuilder::new(&pattern)
            .case_insensitive(!opts.case_sensitive)
            .build()
            .map_err(|e| format!("{TOOL_NAME}: invalid regular expression: {e}"))?;
        let mut hits = Vec::new();
        for caps in re.captures_iter(text) {
            let whole = caps.get(0).expect("group 0 always present");
            // An empty match (e.g. `a*` between letters) replaces nothing
            // useful and would insert text at every position.
            if whole.start() == whole.end() {
                continue;
            }
            let new = if opts.regex {
                let mut dst = String::new();
                caps.expand(opts.replace, &mut dst);
                dst
            } else {
                opts.replace.to_string()
            };
            hits.push(Hit {
                start: whole.start(),
                end: whole.end(),
                new,
            });
        }
        return Ok(hits);
    }

    // Literal, case-sensitive: verbatim, then quote-folded, then CRLF/LF swap.
    let exact: Vec<Hit> = text
        .match_indices(opts.find)
        .map(|(i, m)| Hit {
            start: i,
            end: i + m.len(),
            new: opts.replace.to_string(),
        })
        .collect();
    if !exact.is_empty() {
        return Ok(exact);
    }
    let folded = literal_folded_hits(text, opts.find, opts.replace);
    if !folded.is_empty() {
        return Ok(folded);
    }
    if let Some(swapped) = newline_swapped(opts.find) {
        let new = align_newlines(opts.replace, opts.find, &swapped);
        let exact: Vec<Hit> = text
            .match_indices(swapped.as_str())
            .map(|(i, m)| Hit {
                start: i,
                end: i + m.len(),
                new: new.clone(),
            })
            .collect();
        if !exact.is_empty() {
            return Ok(exact);
        }
        return Ok(literal_folded_hits(text, &swapped, &new));
    }
    Ok(Vec::new())
}

fn literal_folded_hits(text: &str, find: &str, replace: &str) -> Vec<Hit> {
    find_quote_folded_ranges(text, find)
        .into_iter()
        .map(|(start, end)| Hit {
            start,
            end,
            new: remap_quotes_to_sample(replace, &text[start..end]),
        })
        .collect()
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

fn apply_hits(text: &str, hits: &[Hit]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    for hit in hits {
        out.push_str(&text[last..hit.start]);
        out.push_str(&hit.new);
        last = hit.end;
    }
    out.push_str(&text[last..]);
    out
}

fn line_of(text: &str, byte: usize) -> usize {
    text[..byte].matches('\n').count() + 1
}

fn context_before(text: &str, byte: usize) -> String {
    let chars: Vec<char> = text[..byte].chars().rev().take(CONTEXT_CHARS).collect();
    chars.into_iter().rev().collect()
}

fn context_after(text: &str, byte: usize) -> String {
    text[byte..].chars().take(CONTEXT_CHARS).collect()
}

fn describe_hits(text: &str, hits: &[Hit]) -> Vec<Value> {
    hits.iter()
        .take(MAX_LISTED_MATCHES)
        .map(|h| {
            json!({
                "line": line_of(text, h.start),
                "before": context_before(text, h.start),
                "matched": &text[h.start..h.end],
                "replacement": h.new,
                "after": context_after(text, h.end),
            })
        })
        .collect()
}

impl Tool for FindReplaceTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, input: &Value) -> AppResult<()> {
        let path = string_arg(input, "path")?;
        if path.is_empty() {
            return Err(invalid("`path` must be non-empty"));
        }
        if string_arg(input, "find")?.is_empty() {
            return Err(invalid("`find` must be non-empty"));
        }
        string_arg(input, "replace")?;
        for key in ["regex", "case_sensitive", "dry_run"] {
            match input.get(key) {
                None | Some(Value::Null) => {}
                Some(v) if v.is_boolean() => {}
                Some(_) => return Err(invalid(&format!("`{key}` must be a boolean"))),
            }
        }
        match input.get("max_replacements") {
            None | Some(Value::Null) => {}
            Some(v) if v.as_u64().is_some_and(|n| n >= 1) => {}
            Some(_) => return Err(invalid("`max_replacements` must be a positive integer")),
        }
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let input = &invocation.input;
            let path = resolve_path(input, &invocation.context.cwd)?;
            crate::ai::agent::tools::pe_docs::refuse_nondoc(TOOL_NAME, &path)?;

            let find = str_of(input, "find");
            let replace = str_of(input, "replace");
            let opts = Options {
                find,
                replace,
                regex: bool_of(input, "regex", false),
                case_sensitive: bool_of(input, "case_sensitive", true),
            };
            let dry_run = bool_of(input, "dry_run", false);
            let max = input
                .get("max_replacements")
                .and_then(Value::as_u64)
                .map(|n| n as usize);

            let decoded = read_text_file(&path)
                .map_err(|e| AppError::Other(format!("{TOOL_NAME}: read {:?}: {e}", path)))?;

            let mut hits = match find_hits(&decoded.text, &opts) {
                Ok(h) => h,
                Err(msg) => return Ok(ToolResult::error(msg)),
            };
            if hits.is_empty() {
                clear_receipt(&invocation.context.read_file_state, &path);
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: `find` matched nothing in {}. Check spelling, letter case \
                     (`case_sensitive`) and whether `regex` should be set.",
                    path.display()
                )));
            }
            let total_matches = hits.len();
            if let Some(max) = max {
                hits.truncate(max);
            }
            let matches = describe_hits(&decoded.text, &hits);

            if dry_run {
                return Ok(ToolResult::ok(json!({
                    "success": true,
                    "dry_run": true,
                    "path": crate::data::paths::normalized_path_key(&path),
                    "total_matches": total_matches,
                    "would_replace": hits.len(),
                    "matches": matches,
                    "listed": matches.len(),
                })));
            }

            let text_before = decoded.text.clone();
            let updated = apply_hits(&text_before, &hits);
            if updated == text_before {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: replacement is identical to the matched text — nothing to change"
                )));
            }

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
                .map_err(|e| AppError::Other(format!("{TOOL_NAME}: write {:?}: {e}", path)))?;
            record_receipt(&invocation.context.read_file_state, &path, &updated, true);

            let path_str = crate::data::paths::normalized_path_key(&path);
            let mut pending_diff_id: Option<String> = None;
            if let (Some(pool), Some(sid)) = (&self.pool, invocation.context.session_id.as_deref())
            {
                if let Ok(conn) = pool.get() {
                    let first = &hits[0];
                    match pending_diff::insert(
                        &conn,
                        sid,
                        &path_str,
                        &text_before[first.start..first.end],
                        &first.new,
                        &text_before,
                        &updated,
                        Some(decoded.encoding.label()),
                        decoded.had_bom,
                        invocation.context.correlation_id.as_deref(),
                    ) {
                        Ok(Some(id)) => pending_diff_id = Some(id),
                        Ok(None) => {}
                        Err(e) => eprintln!("{TOOL_NAME}: failed to record pending diff: {e}"),
                    }
                }
            }

            let mut body = json!({
                "success": true,
                "dry_run": false,
                "path": path_str,
                "replaced_count": hits.len(),
                "total_matches": total_matches,
                "matches": matches,
                "listed": matches.len(),
                "chars": updated.chars().filter(|c| !c.is_whitespace()).count(),
            });
            if let Some(id) = pending_diff_id {
                body["pending_diff_id"] = json!(id);
            }
            Ok(ToolResult::ok(body))
        })
    }
}

fn invalid(msg: &str) -> AppError {
    AppError::Invalid(format!("{TOOL_NAME}: {msg}"))
}

fn string_arg<'a>(input: &'a Value, key: &str) -> AppResult<&'a str> {
    input
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(&format!("`{key}` must be a string")))
}

fn str_of<'a>(input: &'a Value, key: &str) -> &'a str {
    input.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn bool_of(input: &Value, key: &str, default: bool) -> bool {
    input.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn resolve_path(input: &Value, cwd: &Path) -> AppResult<PathBuf> {
    let raw = input
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("missing path"))?;
    project_path::resolve_project_file(cwd, raw, TOOL_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::agent::core::context::{ToolUseContext, ToolUseContextBuilder};
    use crate::ai::agent::tools::text_decode::detect_and_decode;
    use crate::ai::agent::types::{AgentId, MessageId};
    use std::sync::atomic::{AtomicU32, Ordering};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn test_dir() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("moyan-findreplace-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed_bytes(bytes: &[u8]) -> (Arc<ToolUseContext>, String) {
        let dir = test_dir();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let name = format!("doc-{n}.txt");
        std::fs::write(dir.join(&name), bytes).unwrap();
        let ctx = ToolUseContextBuilder::new(AgentId::new(), dir).build().0;
        (ctx, name)
    }

    fn seed(text: &str) -> (Arc<ToolUseContext>, String) {
        seed_bytes(text.as_bytes())
    }

    async fn run(ctx: &Arc<ToolUseContext>, input: Value) -> ToolResult {
        let tool = FindReplaceTool::new(Arc::new(FileSnapshotStore::new()));
        tool.validate(&input).unwrap();
        tool.execute(ToolInvocation {
            id: MessageId("fr".into()),
            input,
            context: ctx.as_ref(),
        })
        .await
        .unwrap()
    }

    fn disk(ctx: &Arc<ToolUseContext>, name: &str) -> String {
        detect_and_decode(&std::fs::read(ctx.cwd.join(name)).unwrap()).text
    }

    #[test]
    fn schema_declares_find_before_replace() {
        let tool = FindReplaceTool::new(Arc::new(FileSnapshotStore::new()));
        let keys: Vec<String> = tool.spec().schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(
            keys,
            vec![
                "path",
                "find",
                "replace",
                "regex",
                "case_sensitive",
                "max_replacements",
                "dry_run"
            ]
        );
    }

    #[test]
    fn validate_rejects_bad_arguments() {
        let tool = FindReplaceTool::new(Arc::new(FileSnapshotStore::new()));
        assert!(tool
            .validate(&json!({ "path": "a.txt", "find": "", "replace": "x" }))
            .is_err());
        assert!(tool
            .validate(&json!({ "path": "a.txt", "find": "a" }))
            .is_err());
        assert!(tool
            .validate(&json!({ "path": "a.txt", "find": "a", "replace": "b", "max_replacements": 0 }))
            .is_err());
        assert!(tool
            .validate(&json!({ "path": "a.txt", "find": "a", "replace": "b", "regex": "yes" }))
            .is_err());
        assert!(tool
            .validate(&json!({ "path": "a.txt", "find": "a", "replace": "b" }))
            .is_ok());
    }

    #[tokio::test]
    async fn replaces_every_literal_occurrence() {
        let (ctx, name) = seed("甲乙甲\n丙甲");
        let res = run(&ctx, json!({ "path": name, "find": "甲", "replace": "X" })).await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "X乙X\n丙X");
        assert_eq!(res.content["replaced_count"], 3);
        assert_eq!(res.content["total_matches"], 3);
    }

    #[tokio::test]
    async fn max_replacements_limits_in_file_order() {
        let (ctx, name) = seed("a a a a");
        let res = run(
            &ctx,
            json!({ "path": name, "find": "a", "replace": "b", "max_replacements": 2 }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "b b a a");
        assert_eq!(res.content["replaced_count"], 2);
        assert_eq!(res.content["total_matches"], 4);
    }

    #[tokio::test]
    async fn case_insensitive_literal() {
        let (ctx, name) = seed("Foo foo FOO");
        let res = run(
            &ctx,
            json!({ "path": name, "find": "foo", "replace": "bar", "case_sensitive": false }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "bar bar bar");
    }

    #[tokio::test]
    async fn case_sensitive_by_default() {
        let (ctx, name) = seed("Foo foo");
        let res = run(&ctx, json!({ "path": name, "find": "foo", "replace": "bar" })).await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "Foo bar");
    }

    #[tokio::test]
    async fn regex_with_capture_groups() {
        let (ctx, name) = seed("第1章 第23章");
        let res = run(
            &ctx,
            json!({ "path": name, "find": r"第(\d+)章", "replace": "Chapter $1", "regex": true }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "Chapter 1 Chapter 23");
    }

    #[tokio::test]
    async fn literal_mode_does_not_interpret_dollar_or_regex_syntax() {
        let (ctx, name) = seed("a.b a+b");
        let res = run(
            &ctx,
            json!({ "path": name, "find": "a.b", "replace": "$1" }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "$1 a+b");
    }

    #[tokio::test]
    async fn invalid_regex_is_an_error_and_writes_nothing() {
        let (ctx, name) = seed("abc");
        let res = run(
            &ctx,
            json!({ "path": name, "find": "(", "replace": "x", "regex": true }),
        )
        .await;
        assert!(res.is_error);
        assert_eq!(disk(&ctx, &name), "abc");
    }

    #[tokio::test]
    async fn dry_run_lists_matches_without_writing() {
        let (ctx, name) = seed("one\ntwo cat\nthree cat");
        let res = run(
            &ctx,
            json!({ "path": name, "find": "cat", "replace": "dog", "dry_run": true }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "one\ntwo cat\nthree cat");
        assert_eq!(res.content["dry_run"], true);
        assert_eq!(res.content["total_matches"], 2);
        assert_eq!(res.content["matches"][0]["line"], 2);
        assert_eq!(res.content["matches"][1]["line"], 3);
        assert_eq!(res.content["matches"][0]["replacement"], "dog");
    }

    #[tokio::test]
    async fn no_match_is_an_error() {
        let (ctx, name) = seed("abc");
        let res = run(&ctx, json!({ "path": name, "find": "zzz", "replace": "x" })).await;
        assert!(res.is_error);
        assert_eq!(disk(&ctx, &name), "abc");
    }

    #[tokio::test]
    async fn identical_replacement_is_an_error() {
        let (ctx, name) = seed("abc");
        let res = run(&ctx, json!({ "path": name, "find": "b", "replace": "b" })).await;
        assert!(res.is_error);
    }

    #[tokio::test]
    async fn empty_replace_deletes_matches() {
        let (ctx, name) = seed("a-b-c");
        let res = run(&ctx, json!({ "path": name, "find": "-", "replace": "" })).await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "abc");
    }

    #[tokio::test]
    async fn crlf_file_keeps_crlf_with_lf_locator() {
        let (ctx, name) = seed("A\r\nB\r\nC");
        let res = run(
            &ctx,
            json!({ "path": name, "find": "A\nB", "replace": "X\nY" }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "X\r\nY\r\nC");
    }

    #[tokio::test]
    async fn curly_quotes_match_ascii_quotes() {
        let (ctx, name) = seed("他说\"你好\"。他又说\"你好\"。");
        let res = run(
            &ctx,
            json!({
                "path": name,
                "find": "\u{201C}你好\u{201D}",
                "replace": "\u{201C}再见\u{201D}",
            }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "他说\"再见\"。他又说\"再见\"。");
        assert_eq!(res.content["replaced_count"], 2);
    }

    #[tokio::test]
    async fn bom_and_encoding_are_preserved() {
        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice("甲乙".as_bytes());
        let (ctx, name) = seed_bytes(&bytes);
        let res = run(&ctx, json!({ "path": name, "find": "甲", "replace": "丙" })).await;
        assert!(!res.is_error, "{:?}", res.content);
        let raw = std::fs::read(ctx.cwd.join(&name)).unwrap();
        assert_eq!(&raw[..3], &[0xEF, 0xBB, 0xBF]);
        assert_eq!(&raw[3..], "丙乙".as_bytes());
    }

    #[test]
    fn hits_skip_empty_regex_matches() {
        let opts = Options {
            find: "x*",
            replace: "-",
            regex: true,
            case_sensitive: true,
        };
        let hits = find_hits("axxb", &opts).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].start, hits[0].end), (1, 3));
    }
}
