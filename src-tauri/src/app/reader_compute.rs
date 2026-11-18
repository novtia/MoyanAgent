//! Offloaded reader text computations: word counting, paragraph chunking,
//! pending-diff hunk placement, and word/sentence-level Myers inline diff.
//! Running these in Rust on Tokio's blocking pool keeps the WebView JS main
//! thread free for rendering and input even on 100k+ char novels.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Unified document word count matching frontend `countWords`:
/// every non-whitespace Unicode scalar counts as 1 word.
pub(crate) fn count_words_str(text: &str) -> usize {
    text.chars().filter(|c| !c.is_whitespace()).count()
}

fn split_lf_lines(text: &str) -> Vec<&str> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect()
}

fn strip_paragraph_label_prefix(s: &str) -> &str {
    let trimmed = s.trim_start();
    let Some(rest) = trimmed.strip_prefix("[P") else {
        return s;
    };
    let Some(close_idx) = rest.find(']') else {
        return s;
    };
    let digits = &rest[..close_idx];
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return s;
    }
    rest[close_idx + 1..].trim_start()
}

fn normalize_diff_line(s: &str) -> String {
    strip_paragraph_label_prefix(s).replace('\r', "")
}

fn normalize_diff_text(s: &str) -> String {
    if !s.contains('\r') && !s.contains("[P") {
        return s.to_string();
    }
    let normalized_crlf = s.replace("\r\n", "\n").replace('\r', "\n");
    normalized_crlf
        .split('\n')
        .map(normalize_diff_line)
        .collect::<Vec<_>>()
        .join("\n")
}

fn find_snippet_char_index(haystack: &str, needle: &str, norm_hay: Option<&str>) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    if let Some(byte_idx) = haystack.find(needle) {
        return Some(haystack[..byte_idx].encode_utf16().count());
    }
    let norm_needle = normalize_diff_text(needle);
    if norm_needle.is_empty() {
        return None;
    }
    let owned_norm;
    let hay_norm = match norm_hay {
        Some(v) => v,
        None => {
            owned_norm = normalize_diff_text(haystack);
            &owned_norm
        }
    };
    let byte_idx = hay_norm.find(&norm_needle)?;
    Some(hay_norm[..byte_idx].encode_utf16().count())
}

/// Map a UTF-16 code-unit offset range `[start, end)` in `text` to 0-based inclusive line numbers.
fn utf16_range_to_lines(text: &str, start_u16: usize, end_u16: usize) -> (usize, usize) {
    let bytes = text.as_bytes();
    let total_u16 = text.encode_utf16().count();
    let s = start_u16.min(total_u16);
    let e = end_u16.max(s).min(total_u16);

    let mut line = 0usize;
    let mut u16_pos = 0usize;
    let mut start_line = 0usize;
    let mut found_start = false;

    for ch in text.chars() {
        if !found_start && u16_pos >= s {
            start_line = line;
            found_start = true;
        }
        let u16_len = ch.len_utf16();
        let next_pos = u16_pos + u16_len;
        if found_start && next_pos > e {
            return (start_line, line);
        }
        if ch == '\n' {
            line += 1;
        }
        u16_pos = next_pos;
    }
    let _ = bytes;
    if !found_start {
        start_line = line;
    }
    (start_line, line.max(start_line))
}

fn find_line_subsequence(haystack: &[&str], needle: &[&str]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|w| w.iter().zip(needle.iter()).all(|(a, b)| a == b))
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffBlockInput {
    pub id: String,
    pub before: String,
    pub after: String,
    #[serde(default)]
    pub text_before: String,
    #[serde(default)]
    pub text_after: String,
    pub paragraph_number: Option<usize>,
}

fn map_char_range_through_later_edits(
    start: usize,
    end: usize,
    from_index: usize,
    blocks: &[DiffBlockInput],
) -> (usize, usize) {
    let mut s = start;
    let mut e = end;
    let mut text = blocks
        .get(from_index)
        .map(|b| b.text_after.as_str())
        .unwrap_or("");

    for b in blocks.iter().skip(from_index + 1) {
        let mut at = find_snippet_char_index(text, &b.before, None);
        if at.is_none() && !b.text_before.is_empty() {
            at = find_snippet_char_index(&b.text_before, &b.before, None);
        }
        let Some(at_idx) = at else {
            text = &b.text_after;
            continue;
        };
        let before_len = b.before.encode_utf16().count();
        let after_len = b.after.encode_utf16().count();
        let delta = after_len as isize - before_len as isize;
        let edit_end = at_idx + before_len;

        if e <= at_idx {
            // entirely before
        } else if s >= edit_end {
            s = (s as isize + delta).max(0) as usize;
            e = (e as isize + delta).max(0) as usize;
        } else {
            let new_start = s.min(at_idx);
            let mut new_end = at_idx + after_len;
            if e > edit_end {
                new_end = new_end.max((e as isize + delta).max(0) as usize);
            }
            s = new_start;
            e = new_end;
        }
        text = &b.text_after;
    }

    (s, e)
}

fn place_pending_block_in_tab(
    tab_text: &str,
    tab_norm: &str,
    tab_lines: &[&str],
    blocks: &[DiffBlockInput],
    index: usize,
) -> (usize, usize) {
    let block = &blocks[index];

    if !block.after.trim().is_empty() {
        if let Some(live_idx) = find_snippet_char_index(tab_text, &block.after, Some(tab_norm)) {
            let len_u16 = block.after.encode_utf16().count();
            return utf16_range_to_lines(tab_text, live_idx, live_idx + len_u16);
        }
    }

    // Map from textAfter / textBefore forward through later edits
    let mut mapped: Option<(usize, usize)> = None;
    if !block.after.trim().is_empty() && !block.text_after.is_empty() {
        if let Some(after_idx) = find_snippet_char_index(&block.text_after, &block.after, None) {
            let len_u16 = block.after.encode_utf16().count();
            mapped = Some(map_char_range_through_later_edits(
                after_idx,
                after_idx + len_u16,
                index,
                blocks,
            ));
        }
    }
    if mapped.is_none()
        && !block.before.trim().is_empty()
        && !block.text_before.is_empty()
        && !block.text_after.is_empty()
    {
        if let Some(before_idx) = find_snippet_char_index(&block.text_before, &block.before, None) {
            let text_after_len = block.text_after.encode_utf16().count();
            let after_start = before_idx.min(text_after_len);
            let after_len = block.after.encode_utf16().count();
            let after_end = (after_start + after_len).min(text_after_len);
            mapped = Some(map_char_range_through_later_edits(
                after_start,
                after_end,
                index,
                blocks,
            ));
        }
    }
    if let Some((ms, me)) = mapped {
        return utf16_range_to_lines(tab_text, ms, me);
    }

    let max_line = tab_lines.len().saturating_sub(1);
    let after_lines: Vec<&str> = block.after.split('\n').collect();
    if let Some(idx) = find_line_subsequence(tab_lines, &after_lines) {
        return (idx, (idx + after_lines.len().saturating_sub(1)).min(max_line));
    }

    let before_lines: Vec<&str> = block.before.split('\n').collect();
    if let Some(idx) = find_line_subsequence(tab_lines, &before_lines) {
        return (idx, (idx + before_lines.len().saturating_sub(1)).min(max_line));
    }

    let fallback_start = block
        .paragraph_number
        .map(|p| p.saturating_sub(1).min(max_line))
        .unwrap_or(0);
    let approx_lines = after_lines.len().max(1);
    (
        fallback_start,
        (fallback_start + approx_lines - 1).min(max_line),
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineDiffAddedRange {
    pub from: usize,
    pub to: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineDiffRemovedSpan {
    pub pos: usize,
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InlineWordDiffResult {
    pub added: Vec<InlineDiffAddedRange>,
    pub removed: Vec<InlineDiffRemovedSpan>,
}

/// Tokenize text into ASCII words/numbers, CJK/other single chars, punctuation, and newlines.
/// Avoids JS `Intl.Segmenter` overhead while aligning diffs cleanly on words & clauses.
fn tokenize_for_word_diff(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut tokens = Vec::with_capacity(text.len() / 2 + 4);
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\r' && i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
            tokens.push(&text[i..i + 2]);
            i += 2;
        } else if b == b'\n' {
            tokens.push(&text[i..i + 1]);
            i += 1;
        } else if b.is_ascii_alphanumeric() || b == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push(&text[start..i]);
        } else {
            let ch_len = text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1);
            tokens.push(&text[i..i + ch_len]);
            i += ch_len;
        }
    }
    tokens
}

enum DiffOp {
    Equal(usize),
    Delete(usize),
    Insert(usize),
}

/// Myers linear-space / O(ND) diff over interned token IDs, with common prefix/suffix trim.
fn diff_token_ids(old: &[u32], new: &[u32]) -> Vec<DiffOp> {
    let mut prefix = 0usize;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0usize;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid = &old[prefix..old.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];

    let mut ops = Vec::new();
    if prefix > 0 {
        ops.push(DiffOp::Equal(prefix));
    }

    let n = old_mid.len();
    let m = new_mid.len();
    if n == 0 && m > 0 {
        ops.push(DiffOp::Insert(m));
    } else if m == 0 && n > 0 {
        ops.push(DiffOp::Delete(n));
    } else if n > 0 && m > 0 {
        let max_d = n + m;
        // Guard against pathological inputs (>20k tokens) by falling back to replace.
        if max_d > 20_000 {
            ops.push(DiffOp::Delete(n));
            ops.push(DiffOp::Insert(m));
        } else {
            let offset = max_d;
            let mut v = vec![0isize; 2 * max_d + 1];
            let mut trace: Vec<Vec<isize>> = Vec::new();
            let mut found_d = max_d;
            'outer: for d in 0..=max_d {
                let d_isize = d as isize;
                for k in (-d_isize..=d_isize).step_by(2) {
                    let k_idx = (k + offset as isize) as usize;
                    let mut x = if k == -d_isize
                        || (k != d_isize && v[k_idx - 1] < v[k_idx + 1])
                    {
                        v[k_idx + 1]
                    } else {
                        v[k_idx - 1] + 1
                    };
                    let mut y = x - k;
                    while (x as usize) < n && (y as usize) < m && old_mid[x as usize] == new_mid[y as usize]
                    {
                        x += 1;
                        y += 1;
                    }
                    v[k_idx] = x;
                    if (x as usize) >= n && (y as usize) >= m {
                        trace.push(v.clone());
                        found_d = d;
                        break 'outer;
                    }
                }
                trace.push(v.clone());
            }

            let mut x = n as isize;
            let mut y = m as isize;
            let mut rev_ops: Vec<DiffOp> = Vec::new();
            for d in (1..=found_d).rev() {
                let v_prev = &trace[d - 1];
                let d_isize = d as isize;
                let k = x - y;
                let prev_k = if k == -d_isize
                    || (k != d_isize
                        && v_prev[(k - 1 + offset as isize) as usize]
                            < v_prev[(k + 1 + offset as isize) as usize])
                {
                    k + 1
                } else {
                    k - 1
                };
                let prev_x = v_prev[(prev_k + offset as isize) as usize];
                let prev_y = prev_x - prev_k;

                while x > prev_x && y > prev_y {
                    rev_ops.push(DiffOp::Equal(1));
                    x -= 1;
                    y -= 1;
                }
                if x == prev_x {
                    rev_ops.push(DiffOp::Insert(1));
                    y -= 1;
                } else {
                    rev_ops.push(DiffOp::Delete(1));
                    x -= 1;
                }
            }
            while x > 0 && y > 0 {
                rev_ops.push(DiffOp::Equal(1));
                x -= 1;
                y -= 1;
            }
            rev_ops.reverse();
            ops.extend(rev_ops);
        }
    }

    if suffix > 0 {
        ops.push(DiffOp::Equal(suffix));
    }
    ops
}

pub(crate) fn compute_inline_word_diff(old_text: &str, new_text: &str) -> InlineWordDiffResult {
    if old_text == new_text {
        return InlineWordDiffResult {
            added: Vec::new(),
            removed: Vec::new(),
        };
    }

    let old_tokens = tokenize_for_word_diff(old_text);
    let new_tokens = tokenize_for_word_diff(new_text);

    let mut intern: HashMap<&str, u32> = HashMap::with_capacity(old_tokens.len() + new_tokens.len());
    let mut next_id = 0u32;
    let old_ids: Vec<u32> = old_tokens
        .iter()
        .map(|&t| {
            *intern.entry(t).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            })
        })
        .collect();
    let new_ids: Vec<u32> = new_tokens
        .iter()
        .map(|&t| {
            *intern.entry(t).or_insert_with(|| {
                let id = next_id;
                next_id += 1;
                id
            })
        })
        .collect();
    let ops = diff_token_ids(&old_ids, &new_ids);

    let mut added: Vec<InlineDiffAddedRange> = Vec::new();
    let mut removed: Vec<InlineDiffRemovedSpan> = Vec::new();
    let mut old_idx = 0usize;
    let mut new_idx = 0usize;
    let mut cursor_u16 = 0usize;

    for op in ops {
        match op {
            DiffOp::Equal(count) => {
                for tok in &new_tokens[new_idx..new_idx + count] {
                    cursor_u16 += tok.encode_utf16().count();
                }
                old_idx += count;
                new_idx += count;
            }
            DiffOp::Delete(count) => {
                let deleted_text: String = old_tokens[old_idx..old_idx + count].concat();
                if !deleted_text.is_empty() {
                    if let Some(prev) = removed.last_mut() {
                        if prev.pos == cursor_u16 {
                            prev.text.push_str(&deleted_text);
                        } else {
                            removed.push(InlineDiffRemovedSpan {
                                pos: cursor_u16,
                                text: deleted_text,
                            });
                        }
                    } else {
                        removed.push(InlineDiffRemovedSpan {
                            pos: cursor_u16,
                            text: deleted_text,
                        });
                    }
                }
                old_idx += count;
            }
            DiffOp::Insert(count) => {
                let mut ins_u16 = 0usize;
                for tok in &new_tokens[new_idx..new_idx + count] {
                    ins_u16 += tok.encode_utf16().count();
                }
                if ins_u16 > 0 {
                    let from = cursor_u16;
                    let to = cursor_u16 + ins_u16;
                    if let Some(prev) = added.last_mut() {
                        if prev.to == from {
                            prev.to = to;
                        } else {
                            added.push(InlineDiffAddedRange { from, to });
                        }
                    } else {
                        added.push(InlineDiffAddedRange { from, to });
                    }
                    cursor_u16 = to;
                }
                new_idx += count;
            }
        }
    }

    InlineWordDiffResult { added, removed }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkRangeDto {
    pub index: usize,
    pub start_line: usize,
    pub end_line: usize,
    pub chars: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDiffLineRangeDto {
    pub block_id: String,
    pub start_line: usize,
    pub end_line: usize,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HunkSegmentDto {
    pub block_id: String,
    pub block_ids: Vec<String>,
    pub before: String,
    pub after: String,
    pub tab_start: usize,
    pub tab_end: usize,
    pub paragraph_number: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PureDeleteDto {
    pub block_id: String,
    pub before: String,
    pub tab_start: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkDiffInfoDto {
    pub chunk_index: usize,
    pub inline_diff: InlineWordDiffResult,
    pub pure_deletes: Vec<PureDeleteDto>,
    pub range: Option<PendingDiffLineRangeDto>,
    pub hunk_index: isize,
    pub block_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReaderChunkLayoutDto {
    pub total_chars: usize,
    pub line_count: usize,
    pub chunks: Vec<ChunkRangeDto>,
    pub line_ranges: Vec<PendingDiffLineRangeDto>,
    pub chunk_diffs: Vec<ChunkDiffInfoDto>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExistingChunkRange {
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComputeReaderLayoutArgs {
    pub text: String,
    pub chunk_chars: usize,
    pub blocks: Vec<DiffBlockInput>,
    #[serde(default)]
    pub pinned_ranges: Option<Vec<ExistingChunkRange>>,
    #[serde(default)]
    pub prev_line_count: Option<usize>,
    #[serde(default)]
    pub focused_chunk_idx: Option<usize>,
}

fn compute_layout_inner(args: ComputeReaderLayoutArgs) -> ReaderChunkLayoutDto {
    let lines = split_lf_lines(&args.text);
    let total_chars = count_words_str(&args.text);
    let line_count = lines.len();
    let tab_norm = normalize_diff_text(&args.text);

    // 1. Place pending diff blocks in current tabText
    let line_ranges: Vec<PendingDiffLineRangeDto> = args
        .blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let (start_line, end_line) =
                place_pending_block_in_tab(&args.text, &tab_norm, &lines, &args.blocks, index);
            PendingDiffLineRangeDto {
                block_id: block.id.clone(),
                start_line,
                end_line,
                before: block.before.clone(),
                after: block.after.clone(),
            }
        })
        .collect();

    // 2. Build merged hunk segments (filtering out semantic no-ops)
    let mut order: HashMap<&str, usize> = HashMap::with_capacity(args.blocks.len());
    let mut block_by_id: HashMap<&str, &DiffBlockInput> = HashMap::with_capacity(args.blocks.len());
    for (i, b) in args.blocks.iter().enumerate() {
        order.insert(b.id.as_str(), i);
        block_by_id.insert(b.id.as_str(), b);
    }

    let mut sorted_indices: Vec<usize> = (0..line_ranges.len()).collect();
    sorted_indices.sort_by(|&a, &b| {
        let ra = &line_ranges[a];
        let rb = &line_ranges[b];
        ra.start_line.cmp(&rb.start_line).then_with(|| {
            order
                .get(ra.block_id.as_str())
                .copied()
                .unwrap_or(0)
                .cmp(&order.get(rb.block_id.as_str()).copied().unwrap_or(0))
        })
    });

    let mut hunks: Vec<HunkSegmentDto> = Vec::new();
    let mut idx_pos = 0usize;
    while idx_pos < sorted_indices.len() {
        let first = &line_ranges[sorted_indices[idx_pos]];
        let mut group = vec![first];
        let group_start = first.start_line;
        let mut group_end = first.end_line;
        let mut j = idx_pos + 1;
        while j < sorted_indices.len() && line_ranges[sorted_indices[j]].start_line <= group_end {
            let cand = &line_ranges[sorted_indices[j]];
            group.push(cand);
            group_end = group_end.max(cand.end_line);
            j += 1;
        }

        let seg = if group.len() == 1 {
            HunkSegmentDto {
                block_id: first.block_id.clone(),
                block_ids: vec![first.block_id.clone()],
                before: first.before.clone(),
                after: first.after.clone(),
                tab_start: first.start_line,
                tab_end: first.end_line,
                paragraph_number: block_by_id
                    .get(first.block_id.as_str())
                    .and_then(|b| b.paragraph_number),
            }
        } else {
            let mut members = group;
            members.sort_by_key(|m| order.get(m.block_id.as_str()).copied().unwrap_or(0));
            let earliest = members[0];
            let latest = members[members.len() - 1];
            let before = members
                .iter()
                .find(|m| !m.before.trim().is_empty())
                .map(|m| m.before.clone())
                .unwrap_or_else(|| earliest.before.clone());
            let after = if !latest.after.is_empty() {
                latest.after.clone()
            } else {
                earliest.after.clone()
            };
            HunkSegmentDto {
                block_id: earliest.block_id.clone(),
                block_ids: members.iter().map(|m| m.block_id.clone()).collect(),
                before,
                after,
                tab_start: group_start,
                tab_end: group_end,
                paragraph_number: block_by_id
                    .get(earliest.block_id.as_str())
                    .and_then(|b| b.paragraph_number),
            }
        };

        let is_noop = if seg.block_ids.len() > 1 {
            let live = lines
                .get(seg.tab_start..=seg.tab_end.min(line_count.saturating_sub(1)))
                .map(|s| s.join("\n"))
                .unwrap_or_default();
            normalize_diff_text(&seg.before) == normalize_diff_text(&live)
        } else {
            normalize_diff_text(&seg.before) == normalize_diff_text(&seg.after)
        };
        if !is_noop {
            hunks.push(seg);
        }
        idx_pos = j;
    }

    // 3. Compute or adjust chunk ranges
    let line_words: Vec<usize> = lines.iter().map(|l| count_words_str(l)).collect();
    let last_line = line_count.saturating_sub(1);

    let chunk_bounds: Vec<(usize, usize)> = if let Some(pinned) = args.pinned_ranges {
        let prev_lc = args.prev_line_count.unwrap_or(line_count);
        if prev_lc == line_count {
            pinned
                .into_iter()
                .map(|r| {
                    let s = r.start_line.min(last_line);
                    let e = r.end_line.max(s).min(last_line);
                    (s, e)
                })
                .collect()
        } else {
            let delta = line_count as isize - prev_lc as isize;
            let target_idx = args.focused_chunk_idx.or_else(|| {
                pinned.iter().position(|r| {
                    let max_end = (r.end_line as isize + delta.max(0)).max(0) as usize;
                    hunks
                        .iter()
                        .any(|h| h.tab_start >= r.start_line && h.tab_start <= max_end)
                })
            });
            if let Some(ti) = target_idx.filter(|&ti| ti < pinned.len()) {
                pinned
                    .into_iter()
                    .enumerate()
                    .map(|(i, r)| {
                        if i < ti {
                            (r.start_line.min(last_line), r.end_line.min(last_line))
                        } else if i == ti {
                            let e = (r.end_line as isize + delta).max(r.start_line as isize) as usize;
                            (r.start_line.min(last_line), e.min(last_line))
                        } else {
                            let s = (r.start_line as isize + delta).max(0) as usize;
                            let e = (r.end_line as isize + delta).max(0) as usize;
                            (s.min(last_line), e.max(s).min(last_line))
                        }
                    })
                    .collect()
            } else {
                split_paragraph_bounds(&line_words, args.chunk_chars, &hunks)
            }
        }
    } else {
        split_paragraph_bounds(&line_words, args.chunk_chars, &hunks)
    };

    let chunks: Vec<ChunkRangeDto> = chunk_bounds
        .iter()
        .enumerate()
        .map(|(index, &(start_line, end_line))| {
            let chars: usize = line_words
                .get(start_line..=end_line)
                .map(|s| s.iter().sum())
                .unwrap_or(0);
            ChunkRangeDto {
                index,
                start_line,
                end_line,
                chars,
            }
        })
        .collect();

    // 4. Compute per-chunk inline diffs
    let mut range_by_id: HashMap<&str, (usize, &PendingDiffLineRangeDto)> =
        HashMap::with_capacity(line_ranges.len());
    for (idx, r) in line_ranges.iter().enumerate() {
        range_by_id.insert(r.block_id.as_str(), (idx, r));
    }

    let mut chunk_diffs: Vec<ChunkDiffInfoDto> = Vec::new();
    for c in &chunks {
        let matching: Vec<&HunkSegmentDto> = hunks
            .iter()
            .filter(|h| h.tab_start >= c.start_line && h.tab_end <= c.end_line)
            .collect();
        if matching.is_empty() {
            continue;
        }

        let mut added: Vec<InlineDiffAddedRange> = Vec::new();
        let mut removed: Vec<InlineDiffRemovedSpan> = Vec::new();
        let mut pure_deletes: Vec<PureDeleteDto> = Vec::new();

        for h in &matching {
            let show_delete = !h.before.trim().is_empty();
            let show_insert = !h.after.is_empty();
            if show_delete && !show_insert {
                pure_deletes.push(PureDeleteDto {
                    block_id: h.block_id.clone(),
                    before: h.before.clone(),
                    tab_start: h.tab_start,
                });
                continue;
            }
            let live_insert = lines
                .get(h.tab_start..=h.tab_end.min(last_line))
                .map(|s| s.join("\n"))
                .unwrap_or_default();
            let mut chunk_offset_u16 = 0usize;
            for line in lines.iter().take(h.tab_start).skip(c.start_line) {
                chunk_offset_u16 += line.encode_utf16().count() + 1;
            }
            if show_delete && show_insert {
                let part = compute_inline_word_diff(&h.before, &live_insert);
                for a in part.added {
                    added.push(InlineDiffAddedRange {
                        from: a.from + chunk_offset_u16,
                        to: a.to + chunk_offset_u16,
                    });
                }
                for r in part.removed {
                    removed.push(InlineDiffRemovedSpan {
                        pos: r.pos + chunk_offset_u16,
                        text: r.text,
                    });
                }
            } else {
                added.push(InlineDiffAddedRange {
                    from: chunk_offset_u16,
                    to: chunk_offset_u16 + live_insert.encode_utf16().count(),
                });
            }
        }

        let primary = matching[0];
        let located = range_by_id.get(primary.block_id.as_str());
        let mut seen_ids = HashSet::new();
        let mut block_ids = Vec::new();
        for m in &matching {
            for id in &m.block_ids {
                if seen_ids.insert(id.clone()) {
                    block_ids.push(id.clone());
                }
            }
        }

        chunk_diffs.push(ChunkDiffInfoDto {
            chunk_index: c.index,
            inline_diff: InlineWordDiffResult { added, removed },
            pure_deletes,
            range: located.map(|(_, r)| PendingDiffLineRangeDto {
                block_id: r.block_id.clone(),
                start_line: r.start_line,
                end_line: r.end_line,
                before: r.before.clone(),
                after: r.after.clone(),
            }),
            hunk_index: located.map(|(i, _)| *i as isize).unwrap_or(-1),
            block_ids,
        });
    }

    ReaderChunkLayoutDto {
        total_chars,
        line_count,
        chunks,
        line_ranges,
        chunk_diffs,
    }
}

fn split_paragraph_bounds(
    line_words: &[usize],
    max_chars: usize,
    protected_spans: &[HunkSegmentDto],
) -> Vec<(usize, usize)> {
    if line_words.is_empty() {
        return Vec::new();
    }
    let limit = max_chars.max(1);
    let mut bounds = Vec::new();
    let mut start_line = 0usize;
    let mut acc_chars = line_words[0];

    for (i, &w) in line_words.iter().enumerate().skip(1) {
        let protected = protected_spans
            .iter()
            .any(|s| i - 1 >= s.tab_start && i <= s.tab_end);
        if acc_chars + w <= limit || protected {
            acc_chars += w;
        } else {
            bounds.push((start_line, i - 1));
            start_line = i;
            acc_chars = w;
        }
    }
    bounds.push((start_line, line_words.len() - 1));
    bounds
}

#[tauri::command]
pub async fn compute_reader_chunk_layout(
    args: ComputeReaderLayoutArgs,
) -> Result<ReaderChunkLayoutDto, AppError> {
    tokio::task::spawn_blocking(move || compute_layout_inner(args))
        .await
        .map_err(|e| AppError::Other(format!("compute_reader_chunk_layout join: {e}")))
}

#[tauri::command]
pub async fn compute_inline_word_diff_cmd(
    old_text: String,
    new_text: String,
) -> Result<InlineWordDiffResult, AppError> {
    tokio::task::spawn_blocking(move || compute_inline_word_diff(&old_text, &new_text))
        .await
        .map_err(|e| AppError::Other(format!("compute_inline_word_diff join: {e}")))
}
