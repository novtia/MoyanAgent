//! `FindReplace` — find / replace in a single file, literal or regex.
//!
//! Where `Edit` swaps one unique span, this tool is for bulk work: renaming a
//! term everywhere, regex rewrites with capture groups, capped replacement
//! counts, batch rules (`replacements` or a list of `find` terms) in one pass,
//! and a `dry_run` that lists the hits without touching the file.
//!
//! Literal matching mirrors `Edit`: verbatim first, then quote-folded
//! (ASCII `"` / `“”` / `「」` are the same glyph) and CRLF/LF swaps, so a model
//! that cannot see the file's exact quote style still lands. Regex mode matches
//! as written. Multi-rule calls match simultaneously against the original text
//! (earlier rules win on overlapping spans) and apply all edits in one pass.
//! Writes snapshot the pre-image, keep the file's encoding and BOM,
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
/// Hits listed in a single-rule result; the count still covers all of them.
const MAX_LISTED_MATCHES: usize = 20;
/// Hits listed in a multi-rule batch result.
const MAX_BATCH_LISTED_MATCHES: usize = 40;
/// Most rules accepted in one batched call.
const MAX_BATCH_RULES: usize = 50;
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
                    rewrites; use Edit to rewrite one specific passage. To find or replace \
                    multiple terms/pairs in one call, pass `replacements: [{ find, replace }, ...]` \
                    (or a list of strings in `find`) instead of issuing multiple calls. Call \
                    with `dry_run: true` first when unsure what will match — it lists the hits \
                    (line number plus surrounding text) and writes nothing (and `replace` may \
                    be omitted during `dry_run`). With `regex: true`, `replace` may reference \
                    capture groups as `$1` or `${name}` (write `$$` for a literal dollar sign). \
                    Literal matching treats ASCII and typographic quotes as equivalent. An empty \
                    `replace` deletes the matches."
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
                            "description": "Text to find in single-rule mode (or omit when passing `replacements`). A non-empty string (a list of strings is also accepted to match several terms with the same options). Treated as a regular expression when `regex` is true."
                        },
                        "replace": {
                            "type": "string",
                            "description": "Replacement text for `find` (required unless `replacements` is used or `dry_run` is true). With `regex: true`, `$1` / `${name}` insert capture groups. Empty string deletes the matches."
                        },
                        "replacements": {
                            "type": "array",
                            "minItems": 1,
                            "maxItems": MAX_BATCH_RULES,
                            "description": "Batch mode: list of `{ find, replace, regex?, case_sensitive?, max_replacements? }` rules applied in one pass against the original text (earlier rules win on overlap). Use this to find/replace multiple items at once.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "find": {
                                        "type": "string",
                                        "description": "Text to find (regex when `regex` is true). Non-empty."
                                    },
                                    "replace": {
                                        "type": "string",
                                        "description": "Replacement text. Required unless `dry_run` is true. Empty string deletes the matches."
                                    },
                                    "regex": {
                                        "type": "boolean",
                                        "description": "Override top-level `regex` for this rule."
                                    },
                                    "case_sensitive": {
                                        "type": "boolean",
                                        "description": "Override top-level `case_sensitive` for this rule."
                                    },
                                    "max_replacements": {
                                        "type": "integer",
                                        "minimum": 1,
                                        "description": "Override top-level `max_replacements` for this rule."
                                    }
                                },
                                "required": ["find"]
                            }
                        },
                        "regex": {
                            "type": "boolean",
                            "description": "Treat `find` as a regular expression (default for all rules). Defaults to false (literal text)."
                        },
                        "case_sensitive": {
                            "type": "boolean",
                            "description": "Match letter case exactly (default for all rules). Defaults to true."
                        },
                        "max_replacements": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Replace only the first N matches per rule (in file order). Omit to replace all."
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "List the matches without modifying the file. Defaults to false."
                        }
                    },
                    "required": ["path"]
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
    rule_index: usize,
}

#[derive(Debug, Clone)]
struct Rule {
    find: String,
    replace: String,
    regex: bool,
    case_sensitive: bool,
    max_replacements: Option<usize>,
}

#[derive(Debug)]
struct Options<'a> {
    find: &'a str,
    replace: &'a str,
    regex: bool,
    case_sensitive: bool,
}

/// All hits in file order. Regex errors surface as `Err`.
fn find_hits(text: &str, opts: &Options<'_>, rule_index: usize) -> Result<Vec<Hit>, String> {
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
                rule_index,
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
            rule_index,
        })
        .collect();
    if !exact.is_empty() {
        return Ok(exact);
    }
    let folded = literal_folded_hits(text, opts.find, opts.replace, rule_index);
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
                rule_index,
            })
            .collect();
        if !exact.is_empty() {
            return Ok(exact);
        }
        return Ok(literal_folded_hits(text, &swapped, &new, rule_index));
    }
    Ok(Vec::new())
}

fn literal_folded_hits(text: &str, find: &str, replace: &str, rule_index: usize) -> Vec<Hit> {
    find_quote_folded_ranges(text, find)
        .into_iter()
        .map(|(start, end)| Hit {
            start,
            end,
            new: remap_quotes_to_sample(replace, &text[start..end]),
            rule_index,
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

fn overlaps_any(sorted_ranges: &[(usize, usize)], start: usize, end: usize) -> bool {
    let idx = sorted_ranges.partition_point(|&(s, _)| s < end);
    idx > 0 && sorted_ranges[idx - 1].1 > start
}

fn insert_sorted_range(sorted_ranges: &mut Vec<(usize, usize)>, start: usize, end: usize) {
    let idx = sorted_ranges.partition_point(|&(s, _)| s < start);
    sorted_ranges.insert(idx, (start, end));
}

#[derive(Debug, Default)]
struct RuleStats {
    raw_matches: usize,
    applied_count: usize,
    skipped_overlaps: usize,
}

fn select_listed_hits<'a>(hits: &'a [Hit], num_rules: usize) -> Vec<&'a Hit> {
    if num_rules <= 1 {
        return hits.iter().take(MAX_LISTED_MATCHES).collect();
    }
    if hits.len() <= MAX_BATCH_LISTED_MATCHES {
        return hits.iter().collect();
    }

    let mut chosen = vec![false; hits.len()];
    let mut picked = Vec::with_capacity(MAX_BATCH_LISTED_MATCHES);

    // Guarantee at least one example from every matching rule before filling in file order.
    let mut seen_rule = vec![false; num_rules];
    for (idx, h) in hits.iter().enumerate() {
        if !seen_rule[h.rule_index] {
            seen_rule[h.rule_index] = true;
            chosen[idx] = true;
            picked.push(idx);
            if picked.len() >= MAX_BATCH_LISTED_MATCHES {
                break;
            }
        }
    }
    if picked.len() < MAX_BATCH_LISTED_MATCHES {
        for (idx, _) in hits.iter().enumerate() {
            if chosen[idx] {
                continue;
            }
            chosen[idx] = true;
            picked.push(idx);
            if picked.len() >= MAX_BATCH_LISTED_MATCHES {
                break;
            }
        }
    }
    picked.sort_unstable();
    picked.into_iter().map(|i| &hits[i]).collect()
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

fn describe_hits(text: &str, hits: &[&Hit], include_rule_index: bool) -> Vec<Value> {
    hits.iter()
        .map(|h| {
            let mut item = json!({
                "line": line_of(text, h.start),
                "before": context_before(text, h.start),
                "matched": &text[h.start..h.end],
                "replacement": h.new,
                "after": context_after(text, h.end),
            });
            if include_rule_index {
                item["rule_index"] = json!(h.rule_index);
            }
            item
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
        let _ = parse_rules(input)?;
        Ok(())
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let input = &invocation.input;
            let path = resolve_path(input, &invocation.context.cwd)?;
            crate::ai::agent::tools::pe_docs::refuse_nondoc(TOOL_NAME, &path)?;

            let dry_run = bool_of(input, "dry_run", false);
            let rules = parse_rules(input)?;
            let is_batch = rules.len() > 1;

            let decoded = read_text_file(&path)
                .map_err(|e| AppError::Other(format!("{TOOL_NAME}: read {:?}: {e}", path)))?;

            let mut all_rule_hits: Vec<Vec<Hit>> = Vec::with_capacity(rules.len());
            for (idx, rule) in rules.iter().enumerate() {
                let opts = Options {
                    find: &rule.find,
                    replace: &rule.replace,
                    regex: rule.regex,
                    case_sensitive: rule.case_sensitive,
                };
                let rule_hits = match find_hits(&decoded.text, &opts, idx) {
                    Ok(h) => h,
                    Err(msg) => return Ok(ToolResult::error(msg)),
                };
                all_rule_hits.push(rule_hits);
            }

            let mut accepted_ranges: Vec<(usize, usize)> = Vec::new();
            let mut hits: Vec<Hit> = Vec::new();
            let mut stats: Vec<RuleStats> = Vec::with_capacity(rules.len());

            for (idx, (rule, rule_hits)) in rules.iter().zip(all_rule_hits).enumerate() {
                let mut st = RuleStats {
                    raw_matches: rule_hits.len(),
                    ..RuleStats::default()
                };
                for hit in rule_hits {
                    if overlaps_any(&accepted_ranges, hit.start, hit.end) {
                        st.skipped_overlaps += 1;
                        continue;
                    }
                    if rule
                        .max_replacements
                        .is_some_and(|max| st.applied_count >= max)
                    {
                        break;
                    }
                    insert_sorted_range(&mut accepted_ranges, hit.start, hit.end);
                    st.applied_count += 1;
                    hits.push(hit);
                    let _ = idx;
                }
                stats.push(st);
            }

            if hits.is_empty() {
                clear_receipt(&invocation.context.read_file_state, &path);
                let msg = if is_batch {
                    format!(
                        "{TOOL_NAME}: none of the {} rules matched anything in {}. Check \
                         spelling, letter case (`case_sensitive`) and whether `regex` should be set.",
                        rules.len(),
                        path.display()
                    )
                } else {
                    format!(
                        "{TOOL_NAME}: `find` matched nothing in {}. Check spelling, letter case \
                         (`case_sensitive`) and whether `regex` should be set.",
                        path.display()
                    )
                };
                return Ok(ToolResult::error(msg));
            }

            hits.sort_by_key(|h| h.start);
            let total_matches: usize = stats.iter().map(|s| s.raw_matches).sum();
            let selected = select_listed_hits(&hits, rules.len());
            let matches = describe_hits(&decoded.text, &selected, is_batch);

            let per_rule: Option<Vec<Value>> = if is_batch {
                Some(
                    rules
                        .iter()
                        .zip(&stats)
                        .enumerate()
                        .map(|(i, (rule, st))| {
                            let mut row = json!({
                                "index": i,
                                "find": rule.find,
                                "replace": rule.replace,
                                "total_matches": st.raw_matches,
                            });
                            if dry_run {
                                row["would_replace"] = json!(st.applied_count);
                            } else {
                                row["replaced_count"] = json!(st.applied_count);
                            }
                            if st.skipped_overlaps > 0 {
                                row["skipped_overlaps"] = json!(st.skipped_overlaps);
                            }
                            row
                        })
                        .collect(),
                )
            } else {
                None
            };

            if dry_run {
                let mut body = json!({
                    "success": true,
                    "dry_run": true,
                    "path": crate::data::paths::normalized_path_key(&path),
                    "total_matches": total_matches,
                    "would_replace": hits.len(),
                    "matches": matches,
                    "listed": matches.len(),
                });
                if let Some(pr) = per_rule {
                    body["per_rule"] = json!(pr);
                }
                return Ok(ToolResult::ok(body));
            }

            let text_before = decoded.text.clone();
            let updated = apply_hits(&text_before, &hits);
            if updated == text_before {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: replacement is identical to the matched text — nothing to change"
                )));
            }

            self.snapshots.record(
                invocation.context.owner_session_id(),
                invocation.context.owner_request_id(),
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
            if let (Some(pool), Some(sid)) = (&self.pool, invocation.context.owner_session_id())
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
                        invocation.context.owner_request_id(),
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
            if let Some(pr) = per_rule {
                body["per_rule"] = json!(pr);
            }
            if let Some(id) = pending_diff_id.as_ref() {
                body["pending_diff_id"] = json!(id);
            }
            Ok(ToolResult::ok(body).with_effect(
                crate::ai::agent::tools::WorkspaceEffect::FileUpdated {
                    path: path_str,
                    pending_diff_id,
                    ensure_in_reader: true,
                },
            ))
        })
    }
}

fn parse_optional_max(val: Option<&Value>, label: &str) -> AppResult<Option<usize>> {
    match val {
        None | Some(Value::Null) => Ok(None),
        Some(v) if v.as_u64().is_some_and(|n| n >= 1) => Ok(Some(v.as_u64().unwrap() as usize)),
        Some(_) => Err(invalid(&format!("{label} must be a positive integer"))),
    }
}

fn parse_optional_bool(val: Option<&Value>, default: bool, label: &str) -> AppResult<bool> {
    match val {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(invalid(&format!("{label} must be a boolean"))),
    }
}

fn parse_replace_arg(val: Option<&Value>, dry_run: bool, label: &str) -> AppResult<String> {
    match val {
        Some(Value::String(s)) => Ok(s.clone()),
        None | Some(Value::Null) if dry_run => Ok(String::new()),
        _ => Err(invalid(&format!("{label} must be a string"))),
    }
}

fn parse_rules(input: &Value) -> AppResult<Vec<Rule>> {
    let dry_run = parse_optional_bool(input.get("dry_run"), false, "`dry_run`")?;
    let default_regex = parse_optional_bool(input.get("regex"), false, "`regex`")?;
    let default_case = parse_optional_bool(input.get("case_sensitive"), true, "`case_sensitive`")?;
    let default_max = parse_optional_max(input.get("max_replacements"), "`max_replacements`")?;

    let find_val = input.get("find").filter(|v| !v.is_null());
    let replacements_val = input.get("replacements").filter(|v| !v.is_null());

    match (find_val, replacements_val) {
        (Some(_), Some(_)) => Err(invalid(
            "pass either `find` (+ `replace`) or `replacements`, not both",
        )),
        (None, None) => Err(invalid(
            "must provide either `find` (+ `replace`) or `replacements`",
        )),
        (Some(f), None) => {
            let replace = parse_replace_arg(input.get("replace"), dry_run, "`replace`")?;
            let finds = match f {
                Value::String(s) => {
                    if s.is_empty() {
                        return Err(invalid("`find` must be non-empty"));
                    }
                    vec![s.clone()]
                }
                Value::Array(items) => {
                    if items.is_empty() {
                        return Err(invalid("`find` list must be non-empty"));
                    }
                    if items.len() > MAX_BATCH_RULES {
                        return Err(invalid(&format!(
                            "`find` supports at most {MAX_BATCH_RULES} items per call (got {})",
                            items.len()
                        )));
                    }
                    let mut out = Vec::with_capacity(items.len());
                    for item in items {
                        let s = item
                            .as_str()
                            .ok_or_else(|| invalid("every `find` entry must be a string"))?;
                        if s.is_empty() {
                            return Err(invalid("every `find` entry must be non-empty"));
                        }
                        out.push(s.to_string());
                    }
                    out
                }
                _ => return Err(invalid("`find` must be a string or list of strings")),
            };
            Ok(finds
                .into_iter()
                .map(|find| Rule {
                    find,
                    replace: replace.clone(),
                    regex: default_regex,
                    case_sensitive: default_case,
                    max_replacements: default_max,
                })
                .collect())
        }
        (None, Some(r)) => {
            let items = r.as_array().ok_or_else(|| {
                invalid("`replacements` must be an array of { find, replace } objects")
            })?;
            if items.is_empty() {
                return Err(invalid("`replacements` must contain at least one rule"));
            }
            if items.len() > MAX_BATCH_RULES {
                return Err(invalid(&format!(
                    "`replacements` supports at most {MAX_BATCH_RULES} rules per call (got {})",
                    items.len()
                )));
            }
            let mut rules = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                let obj = item
                    .as_object()
                    .ok_or_else(|| invalid(&format!("`replacements[{i}]` must be an object")))?;
                let find = obj
                    .get("find")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid(&format!("`replacements[{i}].find` must be a string")))?;
                if find.is_empty() {
                    return Err(invalid(&format!(
                        "`replacements[{i}].find` must be non-empty"
                    )));
                }
                let replace = parse_replace_arg(
                    obj.get("replace"),
                    dry_run,
                    &format!("`replacements[{i}].replace`"),
                )?;
                let regex = parse_optional_bool(
                    obj.get("regex"),
                    default_regex,
                    &format!("`replacements[{i}].regex`"),
                )?;
                let case_sensitive = parse_optional_bool(
                    obj.get("case_sensitive"),
                    default_case,
                    &format!("`replacements[{i}].case_sensitive`"),
                )?;
                let max_replacements =
                    if obj.get("max_replacements").is_some_and(|v| !v.is_null()) {
                        parse_optional_max(
                            obj.get("max_replacements"),
                            &format!("`replacements[{i}].max_replacements`"),
                        )?
                    } else {
                        default_max
                    };
                rules.push(Rule {
                    find: find.to_string(),
                    replace,
                    regex,
                    case_sensitive,
                    max_replacements,
                });
            }
            Ok(rules)
        }
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
                "replacements",
                "regex",
                "case_sensitive",
                "max_replacements",
                "dry_run"
            ]
        );
        let item_keys: Vec<String> =
            tool.spec().schema["properties"]["replacements"]["items"]["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect();
        assert_eq!(
            item_keys,
            vec![
                "find",
                "replace",
                "regex",
                "case_sensitive",
                "max_replacements",
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
        let hits = find_hits("axxb", &opts, 0).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!((hits[0].start, hits[0].end), (1, 3));
    }

    #[test]
    fn validate_batch_rules_checks_errors() {
        let tool = FindReplaceTool::new(Arc::new(FileSnapshotStore::new()));
        // Neither `find` nor `replacements`.
        assert!(tool.validate(&json!({ "path": "a.txt" })).is_err());
        // Both `find` and `replacements`.
        assert!(tool
            .validate(&json!({
                "path": "a.txt",
                "find": "a",
                "replace": "b",
                "replacements": [{ "find": "a", "replace": "b" }]
            }))
            .is_err());
        // Empty `replacements`.
        assert!(tool
            .validate(&json!({ "path": "a.txt", "replacements": [] }))
            .is_err());
        // Missing `replace` when `dry_run` is false.
        assert!(tool
            .validate(&json!({
                "path": "a.txt",
                "replacements": [{ "find": "a" }]
            }))
            .is_err());
        // Missing `replace` is allowed when `dry_run` is true.
        assert!(tool
            .validate(&json!({
                "path": "a.txt",
                "replacements": [{ "find": "a" }, { "find": "b" }],
                "dry_run": true
            }))
            .is_ok());
        assert!(tool
            .validate(&json!({
                "path": "a.txt",
                "find": ["a", "b"],
                "dry_run": true
            }))
            .is_ok());
        // Empty `find` entry in list or item.
        assert!(tool
            .validate(&json!({
                "path": "a.txt",
                "find": ["a", ""],
                "replace": "x"
            }))
            .is_err());
        assert!(tool
            .validate(&json!({
                "path": "a.txt",
                "replacements": [{ "find": "", "replace": "x" }]
            }))
            .is_err());
        // Over MAX_BATCH_RULES.
        let too_many: Vec<Value> = (0..=MAX_BATCH_RULES)
            .map(|i| json!({ "find": format!("k{i}"), "replace": "v" }))
            .collect();
        assert!(tool
            .validate(&json!({ "path": "a.txt", "replacements": too_many }))
            .is_err());
    }

    #[tokio::test]
    async fn batch_replacements_apply_in_one_pass_and_support_swap() {
        let (ctx, name) = seed("甲和乙，还有甲乙。");
        let res = run(
            &ctx,
            json!({
                "path": name,
                "replacements": [
                    { "find": "甲", "replace": "乙" },
                    { "find": "乙", "replace": "甲" }
                ]
            }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "乙和甲，还有乙甲。");
        assert_eq!(res.content["replaced_count"], 4);
        assert_eq!(res.content["total_matches"], 4);
        assert_eq!(res.content["per_rule"].as_array().unwrap().len(), 2);
        assert_eq!(res.content["per_rule"][0]["replaced_count"], 2);
        assert_eq!(res.content["per_rule"][1]["replaced_count"], 2);
    }

    #[tokio::test]
    async fn batch_replacements_earlier_rule_wins_on_overlap() {
        let (ctx, name) = seed("张三和三一起去见张三");
        let res = run(
            &ctx,
            json!({
                "path": name,
                "replacements": [
                    { "find": "张三", "replace": "李四" },
                    { "find": "三", "replace": "3" }
                ]
            }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "李四和3一起去见李四");
        assert_eq!(res.content["replaced_count"], 3);
        assert_eq!(res.content["per_rule"][0]["replaced_count"], 2);
        assert_eq!(res.content["per_rule"][1]["total_matches"], 3);
        assert_eq!(res.content["per_rule"][1]["replaced_count"], 1);
        assert_eq!(res.content["per_rule"][1]["skipped_overlaps"], 2);
    }

    #[tokio::test]
    async fn batch_replacements_with_per_rule_options_and_misses() {
        let (ctx, name) = seed("Foo foo 第1章 第2章");
        let res = run(
            &ctx,
            json!({
                "path": name,
                "replacements": [
                    { "find": "foo", "replace": "bar", "case_sensitive": false, "max_replacements": 1 },
                    { "find": r"第(\d+)章", "replace": "Ch.$1", "regex": true },
                    { "find": "not_here", "replace": "x" }
                ]
            }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "bar foo Ch.1 Ch.2");
        assert_eq!(res.content["replaced_count"], 3);
        assert_eq!(res.content["per_rule"][0]["total_matches"], 2);
        assert_eq!(res.content["per_rule"][0]["replaced_count"], 1);
        assert_eq!(res.content["per_rule"][1]["replaced_count"], 2);
        assert_eq!(res.content["per_rule"][2]["total_matches"], 0);
        assert_eq!(res.content["per_rule"][2]["replaced_count"], 0);
    }

    #[tokio::test]
    async fn batch_dry_run_supports_omitted_replace_for_replacements_and_find_list() {
        let (ctx, name) = seed("第一行 张老三\n第二行 老张\n第三行 路人");
        let res = run(
            &ctx,
            json!({
                "path": name,
                "replacements": [
                    { "find": "张老三" },
                    { "find": "老张" },
                    { "find": "王五" }
                ],
                "dry_run": true
            }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(res.content["dry_run"], true);
        assert_eq!(res.content["would_replace"], 2);
        assert_eq!(res.content["matches"][0]["rule_index"], 0);
        assert_eq!(res.content["matches"][0]["line"], 1);
        assert_eq!(res.content["matches"][1]["rule_index"], 1);
        assert_eq!(res.content["matches"][1]["line"], 2);
        assert_eq!(res.content["per_rule"][2]["would_replace"], 0);

        let res_list = run(
            &ctx,
            json!({
                "path": name,
                "find": ["张老三", "老张"],
                "dry_run": true
            }),
        )
        .await;
        assert!(!res_list.is_error, "{:?}", res_list.content);
        assert_eq!(res_list.content["would_replace"], 2);
        // File stays untouched.
        assert_eq!(disk(&ctx, &name), "第一行 张老三\n第二行 老张\n第三行 路人");
    }

    #[tokio::test]
    async fn find_array_matches_multiple_terms_with_shared_replace() {
        let (ctx, name) = seed("张老三来了，老张走了。");
        let res = run(
            &ctx,
            json!({
                "path": name,
                "find": ["张老三", "老张"],
                "replace": "张三"
            }),
        )
        .await;
        assert!(!res.is_error, "{:?}", res.content);
        assert_eq!(disk(&ctx, &name), "张三来了，张三走了。");
        assert_eq!(res.content["replaced_count"], 2);
    }

    #[tokio::test]
    async fn batch_no_matches_returns_error() {
        let (ctx, name) = seed("abc");
        let res = run(
            &ctx,
            json!({
                "path": name,
                "replacements": [
                    { "find": "xxx", "replace": "1" },
                    { "find": "yyy", "replace": "2" }
                ]
            }),
        )
        .await;
        assert!(res.is_error);
        assert_eq!(disk(&ctx, &name), "abc");
    }

    #[tokio::test]
    async fn batch_invalid_regex_writes_nothing() {
        let (ctx, name) = seed("abc");
        let res = run(
            &ctx,
            json!({
                "path": name,
                "replacements": [
                    { "find": "a", "replace": "x" },
                    { "find": "(", "replace": "y", "regex": true }
                ]
            }),
        )
        .await;
        assert!(res.is_error);
        assert_eq!(disk(&ctx, &name), "abc");
    }

    #[tokio::test]
    async fn dry_run_emits_no_workspace_effects_and_replace_emits_file_updated() {
        let (ctx, name) = seed("hello world");
        let preview = run(
            &ctx,
            json!({ "path": name, "find": "world", "replace": "rust", "dry_run": true }),
        )
        .await;
        assert!(!preview.is_error);
        assert!(
            preview.effects.is_empty(),
            "dry_run must not emit workspace sync effects"
        );

        let applied = run(
            &ctx,
            json!({ "path": name, "find": "world", "replace": "rust" }),
        )
        .await;
        assert!(!applied.is_error);
        assert_eq!(applied.effects.len(), 1);
        match &applied.effects[0] {
            crate::ai::agent::tools::WorkspaceEffect::FileUpdated {
                path,
                ensure_in_reader,
                ..
            } => {
                assert!(path.ends_with(&name));
                assert!(*ensure_in_reader);
            }
            other => panic!("expected FileUpdated effect, got {other:?}"),
        }
    }
}
