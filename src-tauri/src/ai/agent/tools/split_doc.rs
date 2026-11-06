//! `SplitDoc` — split one document into several files at paragraph boundaries.
//!
//! The model chooses the cuts. Each number in `at_paragraphs` is the first
//! paragraph of a new file, using the same 1-based line numbers as `Read`
//! (one line = one paragraph). The original file keeps everything before the
//! first cut and its original name. Later pieces are written beside it as
//! `原名（二）`, `原名（三）`, and so on, with the same extension.
//!
//! Slices are byte ranges of the decoded text, so concatenating them restores
//! that text, including `\r\n`. Each piece is written back in the file's
//! original encoding. The original file is shortened last; if that write
//! fails, the new files are removed and the original text is put back.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{json, Value};

use crate::ai::agent::core::file_snapshot::{capture_before, FileOp, FileSnapshotStore};
use crate::ai::agent::tools::project_path::{self, display_path, FILE_REF_DESC};
use crate::ai::agent::tools::read_receipt;
use crate::ai::agent::tools::text_decode::{write_text_file, TextEncoding};
use crate::ai::agent::tools::{Tool, ToolFuture, ToolInvocation, ToolResult, ToolSpec};
use crate::data::db::DbPool;
use crate::data::paths;
use crate::error::{AppError, AppResult};

const TOOL_NAME: &str = "SplitDoc";
const PREVIEW_CHARS: usize = 40;

#[derive(Clone)]
pub struct SplitDocTool {
    spec: ToolSpec,
    snapshots: Arc<FileSnapshotStore>,
    pool: Option<Arc<DbPool>>,
}

impl SplitDocTool {
    pub fn new(snapshots: Arc<FileSnapshotStore>) -> Self {
        Self {
            snapshots,
            pool: None,
            spec: ToolSpec {
                name: TOOL_NAME.to_string(),
                description: "Split one document into several files at paragraph boundaries. \
                    `at_paragraphs` is a list of 1-based paragraph numbers — the same lines \
                    Read reports, one line per paragraph, including a blank line at the end \
                    when the file ends with a newline. Each number is the first paragraph of \
                    a new file. The original file keeps its name and the paragraphs before \
                    the first cut. Example: to turn one chapter into two files with the new \
                    file starting at paragraph 58, pass `[58]`. Numbers must be strictly \
                    increasing and from 2 through the last paragraph. New files are named \
                    `原名（二）`, `原名（三）`, … in the same folder, keeping the extension. \
                    Read the file first and copy the paragraph numbers from that Read; do \
                    not guess the cut, and do not send the prose. This does not rewrite \
                    text or renumber later chapters — use Rename to retitle a piece. Fails \
                    without writing anything if a destination name already exists, a piece \
                    would contain no text, or the file still has unreviewed edits."
                    .to_string(),
                schema: json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": FILE_REF_DESC
                        },
                        "at_paragraphs": {
                            "type": "array",
                            "items": { "type": "integer", "minimum": 2 },
                            "minItems": 1,
                            "description": "1-based paragraph numbers where each new file starts. \
                                Same numbering as Read. Strictly increasing. The original file \
                                keeps every paragraph before the first number. Example: [58] \
                                or [40, 80]."
                        }
                    },
                    "required": ["path", "at_paragraphs"]
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

impl Tool for SplitDocTool {
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
        match input.get("at_paragraphs") {
            Some(Value::Array(items)) if !items.is_empty() => Ok(()),
            Some(Value::Array(_)) => Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `at_paragraphs` must contain at least one paragraph number"
            ))),
            Some(_) => Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `at_paragraphs` must be an array of paragraph numbers, for example [58]"
            ))),
            None => Err(AppError::Invalid(format!(
                "{TOOL_NAME}: `at_paragraphs` is required"
            ))),
        }
    }

    fn execute<'a>(&'a self, invocation: ToolInvocation<'a>) -> ToolFuture<'a> {
        Box::pin(async move {
            let raw = invocation
                .input
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let cwd = &invocation.context.cwd;
            let path = match project_path::resolve_project_file_strict(cwd, raw, TOOL_NAME) {
                Ok(path) => path,
                Err(e) => return Ok(ToolResult::error(e.to_string())),
            };
            if let Err(e) = crate::ai::agent::tools::pe_docs::refuse_nondoc(TOOL_NAME, &path) {
                return Ok(ToolResult::error(e.to_string()));
            }
            if !path.exists() {
                let hint = project_path::resolve_project_file(cwd, raw, TOOL_NAME)
                    .ok()
                    .filter(|found| found != &path)
                    .map(|found| format!(" Did you mean `{}`?", display_path(&found)))
                    .unwrap_or_default();
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: file does not exist: {}.{hint}",
                    display_path(&path)
                )));
            }
            if !path.is_file() {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: not a regular file: {}",
                    display_path(&path)
                )));
            }

            let cuts = match parse_cuts(invocation.input.get("at_paragraphs")) {
                Ok(cuts) => cuts,
                Err(e) => return Ok(ToolResult::error(e)),
            };

            // One read, shared by the slices and the rollback row. An oversized
            // file is refused before its bytes are pulled into memory.
            let captured = capture_before(&path, FileOp::Update)?;
            if !captured.restorable {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: `{}` is too large to snapshot, so it cannot be split",
                    display_path(&path)
                )));
            }
            if !captured.before_existed {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: file does not exist: {}",
                    display_path(&path)
                )));
            }
            let text = captured.before_content.clone().ok_or_else(|| {
                AppError::Other(format!(
                    "{TOOL_NAME}: could not read {}",
                    display_path(&path)
                ))
            })?;
            let encoding = captured.before_encoding.unwrap_or(TextEncoding::Utf8);
            let had_bom = captured.before_had_bom;

            let pieces = match plan_slices(&text, &cuts) {
                Ok(pieces) => pieces,
                Err(e) => return Ok(ToolResult::error(e)),
            };

            let file_name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let parent = path.parent().ok_or_else(|| {
                AppError::Invalid(format!(
                    "{TOOL_NAME}: `{}` has no parent directory",
                    display_path(&path)
                ))
            })?;
            let root = project_path::resolve_project_root(cwd)?;
            let root_canon =
                paths::canonicalize_with_missing_tail(&root).unwrap_or_else(|_| root.clone());
            let parts = match assign_paths(parent, &file_name, &path, &root_canon, pieces) {
                Ok(parts) => parts,
                Err(e) => return Ok(ToolResult::error(e)),
            };

            if self.has_pending_review(invocation.context.session_id.as_deref(), &path)? {
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: `{}` has unreviewed edits. Accept or reject them before splitting.",
                    display_path(&path)
                )));
            }
            for part in parts.iter().skip(1) {
                if part.path.exists() {
                    return Ok(ToolResult::error(format!(
                        "{TOOL_NAME}: `{}` already exists",
                        display_path(&part.path)
                    )));
                }
            }

            let session_id = invocation.context.session_id.as_deref();
            let request_id = invocation.context.correlation_id.as_deref();
            self.snapshots.record(session_id, request_id, &captured)?;
            for part in parts.iter().skip(1) {
                self.snapshots
                    .record_before(session_id, request_id, &part.path, FileOp::Create)?;
            }

            let mut created: Vec<PathBuf> = Vec::new();
            for part in parts.iter().skip(1) {
                if let Err(e) = write_text_file(&part.path, &part.text, encoding, had_bom) {
                    remove_files(&created);
                    let _ = std::fs::remove_file(&part.path);
                    return Err(AppError::Other(format!(
                        "{TOOL_NAME}: write {:?}: {e}",
                        part.path
                    )));
                }
                created.push(part.path.clone());
            }
            if let Err(e) = write_text_file(&path, &parts[0].text, encoding, had_bom) {
                let restored = write_text_file(&path, &text, encoding, had_bom);
                remove_files(&created);
                if restored.is_err() {
                    return Err(AppError::Other(format!(
                        "{TOOL_NAME}: failed to shorten `{}` and could not restore it: {e}",
                        display_path(&path)
                    )));
                }
                return Ok(ToolResult::error(format!(
                    "{TOOL_NAME}: failed to shorten `{}` ({e}). The original file was left unchanged.",
                    display_path(&path)
                )));
            }

            read_receipt::clear_receipt(&invocation.context.read_file_state, &path);

            let paragraphs = parts.last().map(|p| p.paragraph_to).unwrap_or(0);
            let part_json: Vec<Value> = parts
                .iter()
                .map(|part| {
                    let canonical =
                        std::fs::canonicalize(&part.path).unwrap_or_else(|_| part.path.clone());
                    json!({
                        "name": part.name,
                        "path": display_path(&canonical),
                        "paragraph_from": part.paragraph_from,
                        "paragraph_to": part.paragraph_to,
                        "chars": prose_chars(&part.text),
                        "first_line": first_line_preview(&part.text),
                    })
                })
                .collect();
            let canonical_src = std::fs::canonicalize(&path).unwrap_or(path);
            Ok(ToolResult::ok(json!({
                "path": display_path(&canonical_src),
                "paragraphs": paragraphs,
                "parts": part_json,
                "note": "The original file keeps the paragraphs before the first cut. \
                    Rename a part to change its title. Read a part before Edit.",
            })))
        })
    }
}

impl SplitDocTool {
    fn has_pending_review(&self, session_id: Option<&str>, path: &Path) -> AppResult<bool> {
        let (Some(pool), Some(sid)) = (self.pool.as_ref(), session_id) else {
            return Ok(false);
        };
        let conn = pool
            .get()
            .map_err(|e| AppError::Other(format!("{TOOL_NAME}: db connection: {e}")))?;
        let key = paths::normalized_path_key(path);
        let rows = crate::data::pending_diff::list_for_path(&conn, sid, &key)?;
        Ok(!rows.is_empty())
    }
}

#[derive(Debug)]
struct Piece {
    paragraph_from: usize,
    paragraph_to: usize,
    text: String,
}

struct PartPlan {
    name: String,
    path: PathBuf,
    paragraph_from: usize,
    paragraph_to: usize,
    text: String,
}

fn parse_cuts(value: Option<&Value>) -> Result<Vec<usize>, String> {
    let Some(Value::Array(items)) = value else {
        return Err(format!(
            "{TOOL_NAME}: `at_paragraphs` must be an array of paragraph numbers, for example [58]"
        ));
    };
    if items.is_empty() {
        return Err(format!(
            "{TOOL_NAME}: `at_paragraphs` must contain at least one paragraph number"
        ));
    }
    let mut cuts = Vec::with_capacity(items.len());
    for item in items {
        let n = match item.as_u64() {
            Some(n) => n,
            None => {
                return Err(format!(
                    "{TOOL_NAME}: `at_paragraphs` must be integers, for example [58]"
                ));
            }
        };
        let n = usize::try_from(n)
            .map_err(|_| format!("{TOOL_NAME}: paragraph number {n} is too large"))?;
        if n < 2 {
            return Err(format!(
                "{TOOL_NAME}: paragraph {n} is not a cut. Cuts start at paragraph 2 so the original file keeps some text."
            ));
        }
        cuts.push(n);
    }
    Ok(cuts)
}

/// Paragraph start byte offsets. Index 0 is paragraph 1. A trailing newline
/// produces one extra empty paragraph, matching `Read`.
fn paragraph_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0usize];
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            starts.push(i + 1);
        }
    }
    starts
}

fn plan_slices(text: &str, cuts: &[usize]) -> Result<Vec<Piece>, String> {
    if cuts.is_empty() {
        return Err(format!(
            "{TOOL_NAME}: `at_paragraphs` must contain at least one paragraph number"
        ));
    }
    for pair in cuts.windows(2) {
        if pair[1] <= pair[0] {
            return Err(format!(
                "{TOOL_NAME}: `at_paragraphs` must be strictly increasing"
            ));
        }
    }
    let starts = paragraph_starts(text);
    let total = starts.len();
    if total < 2 {
        return Err(format!(
            "{TOOL_NAME}: the file has only {total} paragraph, so there is no boundary to split on"
        ));
    }
    for &cut in cuts {
        if cut < 2 || cut > total {
            return Err(format!(
                "{TOOL_NAME}: paragraph {cut} is out of range. Cuts must be from 2 through {total} \
                 (the file has {total} paragraphs)."
            ));
        }
    }

    let mut pieces = Vec::with_capacity(cuts.len() + 1);
    let mut from_para = 1usize;
    let mut from_byte = 0usize;
    for &cut in cuts {
        let to_byte = starts[cut - 1];
        let slice = text[from_byte..to_byte].to_string();
        let to_para = cut - 1;
        if !has_prose(&slice) {
            return Err(empty_piece_error(from_para, to_para));
        }
        pieces.push(Piece {
            paragraph_from: from_para,
            paragraph_to: to_para,
            text: slice,
        });
        from_para = cut;
        from_byte = to_byte;
    }
    let tail = text[from_byte..].to_string();
    if !has_prose(&tail) {
        return Err(empty_piece_error(from_para, total));
    }
    pieces.push(Piece {
        paragraph_from: from_para,
        paragraph_to: total,
        text: tail,
    });
    Ok(pieces)
}

fn empty_piece_error(from: usize, to: usize) -> String {
    format!(
        "{TOOL_NAME}: paragraphs {from}-{to} have no text. Move the cut so every piece has prose."
    )
}

fn assign_paths(
    dir: &Path,
    original_name: &str,
    original_path: &Path,
    root: &Path,
    pieces: Vec<Piece>,
) -> Result<Vec<PartPlan>, String> {
    let mut parts = Vec::with_capacity(pieces.len());
    for (i, piece) in pieces.into_iter().enumerate() {
        let (name, path) = if i == 0 {
            (original_name.to_string(), original_path.to_path_buf())
        } else {
            let name = part_file_name(original_name, (i + 1) as u32);
            let path = dir.join(&name);
            if path.exists() {
                return Err(format!("{TOOL_NAME}: `{name}` already exists"));
            }
            if let Err(e) = crate::ai::agent::tools::pe_docs::refuse_nondoc(TOOL_NAME, &path) {
                return Err(e.to_string());
            }
            if !paths::is_within(root, &path) {
                return Err(format!(
                    "{TOOL_NAME}: `{name}` resolves outside the project"
                ));
            }
            (name, path)
        };
        parts.push(PartPlan {
            name,
            path,
            paragraph_from: piece.paragraph_from,
            paragraph_to: piece.paragraph_to,
            text: piece.text,
        });
    }
    Ok(parts)
}

/// `第二章.txt` split into a second piece is `第二章（二）.txt`. The count is
/// the piece's place in the series, starting at 2.
fn part_file_name(original_name: &str, index: u32) -> String {
    let (stem, ext) = match original_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => (stem, Some(ext)),
        _ => (original_name, None),
    };
    let titled = format!("{stem}（{}）", chinese_index(index));
    match ext {
        Some(ext) => format!("{titled}.{ext}"),
        None => titled,
    }
}

fn chinese_index(n: u32) -> String {
    const DIGIT: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
    match n {
        0 => "零".into(),
        1..=9 => DIGIT[n as usize].into(),
        10 => "十".into(),
        11..=19 => format!("十{}", DIGIT[(n % 10) as usize]),
        20..=99 => {
            let tens = (n / 10) as usize;
            let ones = (n % 10) as usize;
            if ones == 0 {
                format!("{}十", DIGIT[tens])
            } else {
                format!("{}十{}", DIGIT[tens], DIGIT[ones])
            }
        }
        _ => n.to_string(),
    }
}

fn has_prose(text: &str) -> bool {
    text.chars().any(|c| !c.is_whitespace())
}

fn prose_chars(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

fn first_line_preview(text: &str) -> String {
    let line = text
        .split('\n')
        .map(|line| line.trim_end_matches('\r').trim())
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let mut out = String::new();
    for (i, ch) in line.chars().enumerate() {
        if i == PREVIEW_CHARS {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

fn remove_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::ai::agent::core::context::ToolUseContextBuilder;
    use crate::ai::agent::tools::read_receipt::{is_fresh_write, record_receipt};
    use crate::ai::agent::tools::text_decode::{encode_text, TextEncoding};
    use crate::ai::agent::types::{AgentId, MessageId};

    static COUNTER: AtomicU32 = AtomicU32::new(0);

    fn ctx_in_temp_dir() -> Arc<crate::ai::agent::core::context::ToolUseContext> {
        let dir = std::env::temp_dir().join(format!(
            "moyan-split-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        ToolUseContextBuilder::new(AgentId::new(), dir).build().0
    }

    fn tool() -> SplitDocTool {
        SplitDocTool::new(Arc::new(FileSnapshotStore::new()))
    }

    async fn run(
        ctx: &crate::ai::agent::core::context::ToolUseContext,
        path: &str,
        cuts: &[usize],
    ) -> ToolResult {
        tool()
            .execute(ToolInvocation {
                id: MessageId("split".into()),
                input: json!({ "path": path, "at_paragraphs": cuts }),
                context: ctx,
            })
            .await
            .unwrap()
    }

    #[test]
    fn slices_join_back_to_the_original_including_crlf() {
        let text = "甲\r\n乙\r\n丙";
        let pieces = plan_slices(text, &[3]).unwrap();
        assert_eq!(pieces.len(), 2);
        assert_eq!(pieces[0].text, "甲\r\n乙\r\n");
        assert_eq!(pieces[1].text, "丙");
        assert_eq!(
            pieces.iter().map(|p| p.text.as_str()).collect::<String>(),
            text
        );
        assert_eq!((pieces[0].paragraph_from, pieces[0].paragraph_to), (1, 2));
        assert_eq!((pieces[1].paragraph_from, pieces[1].paragraph_to), (3, 3));
    }

    #[test]
    fn three_cuts_name_the_second_and_third_pieces() {
        assert_eq!(part_file_name("第二章.txt", 2), "第二章（二）.txt");
        assert_eq!(part_file_name("第二章.txt", 3), "第二章（三）.txt");
        assert_eq!(part_file_name("第二章.txt", 10), "第二章（十）.txt");
        assert_eq!(part_file_name("第二章.txt", 11), "第二章（十一）.txt");
        assert_eq!(part_file_name("v1.2 大纲.md", 2), "v1.2 大纲（二）.md");
        assert_eq!(chinese_index(21), "二十一");
    }

    #[test]
    fn bad_cuts_are_rejected_before_any_slice_is_committed() {
        let text = "甲\n\n乙";
        assert!(plan_slices(text, &[3, 2])
            .unwrap_err()
            .contains("strictly increasing"));
        assert!(plan_slices(text, &[9])
            .unwrap_err()
            .contains("out of range"));
        assert!(plan_slices(text, &[1]).is_err());
        let empty_middle = plan_slices(text, &[2, 3]).unwrap_err();
        assert!(empty_middle.contains("no text"), "{empty_middle}");
        assert!(plan_slices("只有一段", &[2])
            .unwrap_err()
            .contains("only 1 paragraph"));
    }

    #[tokio::test]
    async fn splits_on_disk_and_clears_the_read_receipt() {
        let ctx = ctx_in_temp_dir();
        let src = ctx.cwd.join("第二章.txt");
        let text = "甲\r\n乙\r\n丙";
        std::fs::write(&src, text).unwrap();
        record_receipt(&ctx.read_file_state, &src, text, true);

        let result = run(ctx.as_ref(), "第二章.txt", &[2, 3]).await;
        assert!(!result.is_error, "{result:?}");
        assert_eq!(result.content["paragraphs"].as_u64(), Some(3));

        let head = std::fs::read_to_string(&src).unwrap();
        let second = std::fs::read_to_string(ctx.cwd.join("第二章（二）.txt")).unwrap();
        let third = std::fs::read_to_string(ctx.cwd.join("第二章（三）.txt")).unwrap();
        assert_eq!(format!("{head}{second}{third}"), text);
        assert_eq!(head, "甲\r\n");
        assert!(second.contains('\r'));
        assert_eq!(result.content["parts"][0]["chars"].as_u64(), Some(1));
        assert_eq!(
            result.content["parts"][1]["first_line"].as_str(),
            Some("乙")
        );
        assert_eq!(
            result.content["parts"][2]["name"].as_str(),
            Some("第二章（三）.txt")
        );
        assert!(!is_fresh_write(&ctx.read_file_state, &src, text));
        assert!(!is_fresh_write(&ctx.read_file_state, &src, &head));
        let _ = std::fs::remove_dir_all(&ctx.cwd);
    }

    #[tokio::test]
    async fn a_rejected_cut_does_not_touch_the_disk() {
        let ctx = ctx_in_temp_dir();
        let src = ctx.cwd.join("第二章.txt");
        let bytes = "甲\n\n乙".as_bytes();
        std::fs::write(&src, bytes).unwrap();

        let bad = run(ctx.as_ref(), "第二章.txt", &[2, 3]).await;
        assert!(bad.is_error, "{bad:?}");
        assert_eq!(std::fs::read(&src).unwrap(), bytes);
        assert!(!ctx.cwd.join("第二章（二）.txt").exists());

        let range = run(ctx.as_ref(), "第二章.txt", &[9]).await;
        assert!(range.is_error);
        assert_eq!(std::fs::read(&src).unwrap(), bytes);
        let _ = std::fs::remove_dir_all(&ctx.cwd);
    }

    #[tokio::test]
    async fn refuses_when_the_generated_name_already_exists() {
        let ctx = ctx_in_temp_dir();
        std::fs::write(ctx.cwd.join("第二章.txt"), "甲\n乙").unwrap();
        std::fs::write(ctx.cwd.join("第二章（二）.txt"), "已有").unwrap();

        let result = run(ctx.as_ref(), "第二章.txt", &[2]).await;
        assert!(result.is_error, "{result:?}");
        assert!(result.content["error"]
            .as_str()
            .unwrap()
            .contains("already exists"));
        assert_eq!(
            std::fs::read_to_string(ctx.cwd.join("第二章.txt")).unwrap(),
            "甲\n乙"
        );
        assert_eq!(
            std::fs::read_to_string(ctx.cwd.join("第二章（二）.txt")).unwrap(),
            "已有"
        );
        let _ = std::fs::remove_dir_all(&ctx.cwd);
    }

    #[tokio::test]
    async fn gbk_pieces_stay_gbk_and_join_back() {
        let ctx = ctx_in_temp_dir();
        let text = "甲\r\n乙\r\n丙丁戊己";
        let src = ctx.cwd.join("第二章.txt");
        std::fs::write(&src, encode_text(text, TextEncoding::Gbk, false)).unwrap();

        let result = run(ctx.as_ref(), "第二章.txt", &[3]).await;
        assert!(!result.is_error, "{result:?}");

        let head_bytes = std::fs::read(&src).unwrap();
        let tail_bytes = std::fs::read(ctx.cwd.join("第二章（二）.txt")).unwrap();
        assert_eq!(
            head_bytes,
            encode_text("甲\r\n乙\r\n", TextEncoding::Gbk, false)
        );
        assert_eq!(
            tail_bytes,
            encode_text("丙丁戊己", TextEncoding::Gbk, false)
        );
        assert!(std::str::from_utf8(&head_bytes).is_err());
        // Short samples are not always labeled GBK by the charset detector.
        // The bytes themselves are the GBK encoding of each slice.
        assert_eq!(
            encoding_rs::GBK.decode(&head_bytes).0.as_ref(),
            "甲\r\n乙\r\n"
        );
        assert_eq!(encoding_rs::GBK.decode(&tail_bytes).0.as_ref(), "丙丁戊己");
        let _ = std::fs::remove_dir_all(&ctx.cwd);
    }

    #[tokio::test]
    async fn rollback_restores_the_original_and_deletes_the_new_files() {
        let db = crate::data::db::test_support::TempDb::new("split-rollback");
        let dir = std::env::temp_dir().join(format!(
            "moyan-split-rollback-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (ctx, _abort) = ToolUseContextBuilder::new(AgentId::new(), dir.clone())
            .session_id("s-split")
            .correlation_id("u-split")
            .build();
        let src = dir.join("第二章.txt");
        std::fs::write(&src, "甲\n乙\n丙").unwrap();

        let snapshots = Arc::new(FileSnapshotStore::with_pool(Arc::new(db.pool())));
        let result = SplitDocTool::new(snapshots)
            .execute(ToolInvocation {
                id: MessageId("split".into()),
                input: json!({ "path": "第二章.txt", "at_paragraphs": [3] }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(!result.is_error, "{result:?}");
        let dest = dir.join("第二章（二）.txt");
        assert!(dest.is_file());
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "甲\n乙\n");

        let plan = crate::data::file_snapshot::plan_rollback_from_message(
            &db.conn(),
            "s-split",
            "u-split",
        )
        .unwrap()
        .expect("split recorded a rollback plan");
        for restore in &plan.restores {
            let file = PathBuf::from(&restore.path);
            if restore.delete {
                let _ = std::fs::remove_file(&file);
            } else if let Some(content) = &restore.content {
                std::fs::write(&file, content).unwrap();
            }
        }
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "甲\n乙\n丙");
        assert!(!dest.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn refuses_while_a_review_is_open() {
        let db = crate::data::db::test_support::TempDb::new("split-pending");
        let dir = std::env::temp_dir().join(format!(
            "moyan-split-pending-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let (ctx, _abort) = ToolUseContextBuilder::new(AgentId::new(), dir.clone())
            .session_id("s-split")
            .correlation_id("u-split")
            .build();
        let src = dir.join("第二章.txt");
        std::fs::write(&src, "甲\n乙").unwrap();
        let key = paths::normalized_path_key(&src);
        crate::data::pending_diff::insert(
            &db.conn(),
            "s-split",
            &key,
            "甲",
            "甲乙",
            "甲",
            "甲\n乙",
            Some("utf-8"),
            false,
            Some("u-split"),
        )
        .unwrap();

        let result = SplitDocTool::new(Arc::new(FileSnapshotStore::new()))
            .with_pool(Arc::new(db.pool()))
            .execute(ToolInvocation {
                id: MessageId("split".into()),
                input: json!({ "path": "第二章.txt", "at_paragraphs": [2] }),
                context: ctx.as_ref(),
            })
            .await
            .unwrap();
        assert!(result.is_error, "{result:?}");
        assert!(result.content["error"]
            .as_str()
            .unwrap()
            .contains("unreviewed"));
        assert_eq!(std::fs::read_to_string(&src).unwrap(), "甲\n乙");
        assert!(!dir.join("第二章（二）.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
