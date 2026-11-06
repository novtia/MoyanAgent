//! Persistence for pending Edit review hunks (reader Keep/Undo UI).
//!
//! These rows are the authoritative source for the post-apply review UI and
//! survive tab close / app restart until the user accepts or rejects them.
//!
//! They are NOT a rollback source. [`crate::data::file_snapshot`] owns undoing
//! workspace writes; a hunk here is a view of a mutation already recorded
//! there, keyed by the same [`crate::data::paths::normalized_path_key`]. When a
//! message is deleted the snapshot rollback drives disk and these rows are
//! merely retired — two writers would fight, and this one cannot express
//! "delete the file" at all.

use std::path::Path;

use rusqlite::params;
use serde::Serialize;
use ulid::Ulid;

use crate::data::db::{now_ms, DbConn};
use crate::error::AppResult;

/// Max combined UTF-8 bytes of `text_before` + `text_after` we will store.
/// Matches the file-snapshot pre-image cap so huge files skip the review UI.
pub const MAX_PENDING_DIFF_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDiffRow {
    pub id: String,
    pub session_id: String,
    pub path: String,
    pub before_snippet: String,
    pub after_snippet: String,
    pub text_before: String,
    pub text_after: String,
    pub encoding: Option<String>,
    pub had_bom: bool,
    pub seq: i64,
    pub created_at: i64,
    pub request_message_id: Option<String>,
    pub message_id: Option<String>,
}

/// Result of rejecting one or all hunks: content to write back to disk.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingDiffRevert {
    pub path: String,
    pub text: String,
    pub encoding: Option<String>,
    pub had_bom: bool,
}

fn row_from_query(r: &rusqlite::Row<'_>) -> rusqlite::Result<PendingDiffRow> {
    let had_bom: i64 = r.get(8)?;
    Ok(PendingDiffRow {
        id: r.get(0)?,
        session_id: r.get(1)?,
        path: r.get(2)?,
        before_snippet: r.get(3)?,
        after_snippet: r.get(4)?,
        text_before: r.get(5)?,
        text_after: r.get(6)?,
        encoding: r.get(7)?,
        had_bom: had_bom != 0,
        seq: r.get(9)?,
        created_at: r.get(10)?,
        request_message_id: r.get(11)?,
        message_id: r.get(12)?,
    })
}

const SELECT_COLS: &str = "id, session_id, path, before_snippet, after_snippet, \
     text_before, text_after, encoding, had_bom, seq, created_at, \
     request_message_id, message_id";

/// Insert a new pending hunk. Returns `None` when the payloads exceed the
/// size cap (caller should omit `pending_diff_id` from the tool result).
pub fn insert(
    conn: &DbConn,
    session_id: &str,
    path: &str,
    before_snippet: &str,
    after_snippet: &str,
    text_before: &str,
    text_after: &str,
    encoding: Option<&str>,
    had_bom: bool,
    request_message_id: Option<&str>,
) -> AppResult<Option<String>> {
    let total = text_before.len().saturating_add(text_after.len());
    if total > MAX_PENDING_DIFF_BYTES {
        return Ok(None);
    }

    let next_seq: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM pending_diffs
             WHERE session_id = ?1 AND path = ?2",
            params![session_id, path],
            |r| r.get(0),
        )
        .unwrap_or(1);

    let id = Ulid::new().to_string();
    let now = now_ms();
    conn.execute(
        "INSERT INTO pending_diffs(
            id, session_id, path, before_snippet, after_snippet,
            text_before, text_after, encoding, had_bom, seq, created_at,
            request_message_id, message_id)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL)",
        params![
            id,
            session_id,
            path,
            before_snippet,
            after_snippet,
            text_before,
            text_after,
            encoding,
            had_bom as i64,
            next_seq,
            now,
            request_message_id,
        ],
    )?;
    Ok(Some(id))
}

// ---------------------------------------------------------------------------
// Merging overlapping Edits
// ---------------------------------------------------------------------------

/// One contiguous run of changed lines between two texts, found by trimming
/// the common leading and trailing lines. Ends are exclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineSpan {
    start: usize,
    end_before: usize,
    end_after: usize,
}

impl LineSpan {
    fn delta(&self) -> isize {
        self.end_after as isize - self.end_before as isize
    }
}

fn line_span(before: &str, after: &str) -> Option<LineSpan> {
    if before == after {
        return None;
    }
    let b: Vec<&str> = before.split('\n').collect();
    let a: Vec<&str> = after.split('\n').collect();
    let shortest = b.len().min(a.len());
    let mut prefix = 0;
    while prefix < shortest && b[prefix] == a[prefix] {
        prefix += 1;
    }
    let max_suffix = shortest - prefix;
    let mut suffix = 0;
    while suffix < max_suffix && b[b.len() - 1 - suffix] == a[a.len() - 1 - suffix] {
        suffix += 1;
    }
    Some(LineSpan {
        start: prefix,
        end_before: b.len() - suffix,
        end_after: a.len() - suffix,
    })
}

/// Half-open line range.
#[derive(Debug, Clone, Copy)]
struct Range {
    start: usize,
    end: usize,
}

fn shift(v: usize, by: isize) -> usize {
    (v as isize + by).max(0) as usize
}

/// Carry a range from the state before `k` to the state after it.
fn carry_forward(r: Range, k: &LineSpan) -> Range {
    if r.end <= k.start {
        return r;
    }
    if r.start >= k.end_before {
        return Range {
            start: shift(r.start, k.delta()),
            end: shift(r.end, k.delta()),
        };
    }
    // Overlap (only legacy rows can do this): cover both.
    let start = r.start.min(k.start);
    let end = shift(r.end.max(k.end_before), k.delta()).max(start);
    Range { start, end }
}

/// Carry a range from the state after `m` back to the state before it.
/// `None` when the range overlaps what `m` rewrote.
fn back_through(r: Range, m: &LineSpan) -> Option<Range> {
    if r.end <= m.start {
        return Some(r);
    }
    if r.start >= m.end_after {
        return Some(Range {
            start: shift(r.start, -m.delta()),
            end: shift(r.end, -m.delta()),
        });
    }
    None
}

struct MergePlan {
    /// Rows folded into the new hunk (deleted).
    member_ids: Vec<String>,
    /// Surviving later rows whose snapshots no longer contain the folded
    /// edits: (id, text_before, text_after).
    rewritten: Vec<(String, String, String)>,
    /// File text the merged hunk starts from (every surviving row applied).
    merged_before: String,
}

/// Decide whether the Edit `text_before -> text_after` overlaps hunks already
/// pending in this user turn, and if so how to re-chain the rows.
///
/// Rows form an exact chain (each `text_before` is the previous `text_after`),
/// which Undo relies on. Folding row A and the new edit C into one hunk while a
/// disjoint row D sits between them would break that chain, so D is rewritten
/// to apply to the text *without* A, and the merged hunk goes last:
/// `original -> D -> (A + C)`.
fn plan_merge(
    rows: &[PendingDiffRow],
    request_message_id: &str,
    text_before: &str,
    text_after: &str,
) -> Option<MergePlan> {
    let mut start = rows.len();
    while start > 0 && rows[start - 1].request_message_id.as_deref() == Some(request_message_id) {
        start -= 1;
    }
    let tail = &rows[start..];
    if tail.is_empty() {
        return None;
    }
    for pair in tail.windows(2) {
        if pair[1].text_before != pair[0].text_after {
            return None;
        }
    }
    if tail.last()?.text_after != text_before {
        return None;
    }

    let spans: Vec<LineSpan> = tail
        .iter()
        .map(|r| line_span(&r.text_before, &r.text_after))
        .collect::<Option<_>>()?;
    let new_span = line_span(text_before, text_after)?;
    let new_end = new_span.end_before.max(new_span.start + 1);

    let mut member = vec![false; tail.len()];
    for i in 0..tail.len() {
        let mut r = Range {
            start: spans[i].start,
            end: spans[i].end_after,
        };
        for k in (i + 1)..tail.len() {
            r = carry_forward(r, &spans[k]);
        }
        let end = r.end.max(r.start + 1);
        if r.start < new_end && new_span.start < end {
            member[i] = true;
        }
    }
    let first = member.iter().position(|m| *m)?;

    let mut state = tail[first].text_before.clone();
    let mut rewritten = Vec::new();
    for j in first..tail.len() {
        if member[j] {
            continue;
        }
        let mut r = Range {
            start: spans[j].start,
            end: spans[j].end_before,
        };
        for m in (first..j).rev() {
            if member[m] {
                r = back_through(r, &spans[m])?;
            }
        }
        let orig_before: Vec<&str> = tail[j].text_before.split('\n').collect();
        let orig_after: Vec<&str> = tail[j].text_after.split('\n').collect();
        let next_text = {
            let cur: Vec<&str> = state.split('\n').collect();
            if r.start > r.end
                || r.end > cur.len()
                || cur[r.start..r.end] != orig_before[spans[j].start..spans[j].end_before]
            {
                return None;
            }
            let mut next: Vec<&str> = Vec::with_capacity(cur.len());
            next.extend_from_slice(&cur[..r.start]);
            next.extend_from_slice(&orig_after[spans[j].start..spans[j].end_after]);
            next.extend_from_slice(&cur[r.end..]);
            next.join("\n")
        };
        rewritten.push((tail[j].id.clone(), state.clone(), next_text.clone()));
        state = next_text;
    }

    let member_ids = (first..tail.len())
        .filter(|i| member[*i])
        .map(|i| tail[i].id.clone())
        .collect();
    Some(MergePlan {
        member_ids,
        rewritten,
        merged_before: state,
    })
}

fn span_text(lines: &[&str], from: usize, to: usize) -> String {
    lines[from.min(lines.len())..to.min(lines.len())]
        .iter()
        .map(|l| l.trim_end_matches('\r'))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Like [`insert`], but a single-spot Edit that touches lines already covered
/// by a pending hunk of the same user turn is folded into that hunk instead of
/// stacking a second one. The merged hunk spans the union of the lines, shows
/// the text from before the earliest folded edit as "before" and the latest
/// text as "after".
///
/// Falls back to a plain [`insert`] whenever the existing rows cannot be
/// re-chained exactly, so a merge never risks a wrong Undo.
pub fn insert_merging(
    conn: &DbConn,
    session_id: &str,
    path: &str,
    before_snippet: &str,
    after_snippet: &str,
    text_before: &str,
    text_after: &str,
    encoding: Option<&str>,
    had_bom: bool,
    request_message_id: Option<&str>,
) -> AppResult<Option<String>> {
    let plain = || {
        insert(
            conn,
            session_id,
            path,
            before_snippet,
            after_snippet,
            text_before,
            text_after,
            encoding,
            had_bom,
            request_message_id,
        )
    };
    let Some(req) = request_message_id else {
        return plain();
    };
    if text_before.len().saturating_add(text_after.len()) > MAX_PENDING_DIFF_BYTES {
        return plain();
    }
    let rows = list_for_path(conn, session_id, path)?;
    let Some(plan) = plan_merge(&rows, req, text_before, text_after) else {
        return plain();
    };

    let tx = conn.unchecked_transaction()?;
    for (id, before, after) in &plan.rewritten {
        tx.execute(
            "UPDATE pending_diffs SET text_before = ?1, text_after = ?2
             WHERE session_id = ?3 AND id = ?4",
            params![before, after, session_id, id],
        )?;
    }
    for id in &plan.member_ids {
        tx.execute(
            "DELETE FROM pending_diffs WHERE session_id = ?1 AND id = ?2",
            params![session_id, id],
        )?;
    }

    // Edits that cancelled each other out leave nothing to review.
    let Some(span) = line_span(&plan.merged_before, text_after) else {
        tx.commit()?;
        return Ok(None);
    };
    let before_lines: Vec<&str> = plan.merged_before.split('\n').collect();
    let after_lines: Vec<&str> = text_after.split('\n').collect();
    let merged_before_snippet = span_text(&before_lines, span.start, span.end_before);
    let merged_after_snippet = span_text(&after_lines, span.start, span.end_after);

    let next_seq: i64 = tx
        .query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM pending_diffs
             WHERE session_id = ?1 AND path = ?2",
            params![session_id, path],
            |r| r.get(0),
        )
        .unwrap_or(1);
    let id = Ulid::new().to_string();
    tx.execute(
        "INSERT INTO pending_diffs(
            id, session_id, path, before_snippet, after_snippet,
            text_before, text_after, encoding, had_bom, seq, created_at,
            request_message_id, message_id)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL)",
        params![
            id,
            session_id,
            path,
            merged_before_snippet,
            merged_after_snippet,
            plan.merged_before,
            text_after,
            encoding,
            had_bom as i64,
            next_seq,
            now_ms(),
            req,
        ],
    )?;
    tx.commit()?;
    Ok(Some(id))
}

/// Bind unbound rows from this user turn to the finalized assistant message.
pub fn bind_message(
    conn: &DbConn,
    session_id: &str,
    request_message_id: &str,
    assistant_message_id: &str,
) -> AppResult<usize> {
    let n = conn.execute(
        "UPDATE pending_diffs
         SET message_id = ?1
         WHERE session_id = ?2
           AND request_message_id = ?3
           AND message_id IS NULL",
        params![assistant_message_id, session_id, request_message_id],
    )?;
    Ok(n)
}

/// Bind every unbound pending-diff row in the session to an assistant message
/// (used when the originating user message id is unknown, e.g. cancel save).
pub fn bind_unbound(
    conn: &DbConn,
    session_id: &str,
    assistant_message_id: &str,
) -> AppResult<usize> {
    let n = conn.execute(
        "UPDATE pending_diffs
         SET message_id = ?1
         WHERE session_id = ?2 AND message_id IS NULL",
        params![assistant_message_id, session_id],
    )?;
    Ok(n)
}

/// Drop every review row tied to `message_id` (as assistant id or as the
/// originating user `request_message_id`).
///
/// These rows are a review overlay on mutations that `file_snapshots` already
/// owns; deleting the message rolls the disk back from there, so this only has
/// to retire the now-meaningless Keep/Undo entries. Writing `text_before` back
/// here would fight the snapshot rollback and could resurrect a file it had
/// just deleted.
pub fn clear_for_message(conn: &DbConn, session_id: &str, message_id: &str) -> AppResult<usize> {
    let n = conn.execute(
        "DELETE FROM pending_diffs
         WHERE session_id = ?1
           AND (message_id = ?2 OR request_message_id = ?2)",
        params![session_id, message_id],
    )?;
    Ok(n)
}

pub fn list_for_session(conn: &DbConn, session_id: &str) -> AppResult<Vec<PendingDiffRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM pending_diffs
         WHERE session_id = ?1
         ORDER BY path ASC, seq ASC"
    ))?;
    let rows = stmt.query_map(params![session_id], row_from_query)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn list_for_path(
    conn: &DbConn,
    session_id: &str,
    path: &str,
) -> AppResult<Vec<PendingDiffRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM pending_diffs
         WHERE session_id = ?1 AND path = ?2
         ORDER BY seq ASC"
    ))?;
    let rows = stmt.query_map(params![session_id, path], row_from_query)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn get(conn: &DbConn, session_id: &str, id: &str) -> AppResult<Option<PendingDiffRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {SELECT_COLS} FROM pending_diffs
         WHERE session_id = ?1 AND id = ?2"
    ))?;
    let mut rows = stmt.query_map(params![session_id, id], row_from_query)?;
    match rows.next() {
        Some(Ok(row)) => Ok(Some(row)),
        Some(Err(e)) => Err(e.into()),
        None => Ok(None),
    }
}

/// Accept a single hunk: delete that row only.
pub fn accept(conn: &DbConn, session_id: &str, id: &str) -> AppResult<bool> {
    let n = conn.execute(
        "DELETE FROM pending_diffs WHERE session_id = ?1 AND id = ?2",
        params![session_id, id],
    )?;
    Ok(n > 0)
}

/// Reject a hunk: return revert text and delete this row plus every later
/// hunk on the same path (seq >= this row).
pub fn reject(conn: &DbConn, session_id: &str, id: &str) -> AppResult<Option<PendingDiffRevert>> {
    let Some(row) = get(conn, session_id, id)? else {
        return Ok(None);
    };
    conn.execute(
        "DELETE FROM pending_diffs
         WHERE session_id = ?1 AND path = ?2 AND seq >= ?3",
        params![session_id, row.path, row.seq],
    )?;
    Ok(Some(PendingDiffRevert {
        path: row.path,
        text: row.text_before,
        encoding: row.encoding,
        had_bom: row.had_bom,
    }))
}

pub fn accept_all(conn: &DbConn, session_id: &str, path: &str) -> AppResult<usize> {
    let n = conn.execute(
        "DELETE FROM pending_diffs WHERE session_id = ?1 AND path = ?2",
        params![session_id, path],
    )?;
    Ok(n)
}

/// Reject all hunks on a path: revert to the first hunk's text_before.
pub fn reject_all(
    conn: &DbConn,
    session_id: &str,
    path: &str,
) -> AppResult<Option<PendingDiffRevert>> {
    let rows = list_for_path(conn, session_id, path)?;
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    let revert = PendingDiffRevert {
        path: first.path.clone(),
        text: first.text_before.clone(),
        encoding: first.encoding.clone(),
        had_bom: first.had_bom,
    };
    conn.execute(
        "DELETE FROM pending_diffs WHERE session_id = ?1 AND path = ?2",
        params![session_id, path],
    )?;
    Ok(Some(revert))
}

/// Rows are stored under [`crate::data::paths::normalized_path_key`], but a
/// caller may hold a path that no longer resolves the same way — most often
/// after a rollback deleted the file, leaving only the parent directory to
/// canonicalize against, so the file name keeps whatever case it was written
/// with. Match the raw and normalized spellings, case-insensitively where the
/// filesystem itself is.
/// Point review rows for `from` at `to` after a rename.
///
/// Undo writes `text_before` back to the row's path. Leaving the old path
/// would recreate the previous file name beside the renamed one.
pub fn remap_path(conn: &DbConn, session_id: &str, from: &Path, to: &Path) -> AppResult<usize> {
    let from_key = crate::data::paths::normalized_path_key(from);
    let to_key = crate::data::paths::normalized_path_key(to);
    if from_key == to_key {
        return Ok(0);
    }
    let from_raw = crate::data::paths::display_path(from);
    #[cfg(windows)]
    const SQL: &str = "UPDATE pending_diffs SET path = ?1
                       WHERE session_id = ?2
                         AND (path COLLATE NOCASE = ?3 OR path COLLATE NOCASE = ?4)";
    #[cfg(not(windows))]
    const SQL: &str = "UPDATE pending_diffs SET path = ?1
                       WHERE session_id = ?2 AND (path = ?3 OR path = ?4)";
    let n = conn.execute(SQL, params![to_key, session_id, from_raw, from_key])?;
    Ok(n)
}

pub fn clear_path(conn: &DbConn, session_id: &str, path: &str) -> AppResult<()> {
    let normalized = crate::data::paths::normalized_path_key(std::path::Path::new(path));
    #[cfg(windows)]
    const SQL: &str = "DELETE FROM pending_diffs WHERE session_id = ?1
                       AND (path COLLATE NOCASE = ?2 OR path COLLATE NOCASE = ?3)";
    #[cfg(not(windows))]
    const SQL: &str =
        "DELETE FROM pending_diffs WHERE session_id = ?1 AND (path = ?2 OR path = ?3)";
    conn.execute(SQL, params![session_id, path, normalized])?;
    Ok(())
}

/// After a snapshot rollback rewrote `path` to `restored`, keep the review rows
/// that survived (earlier turns) when they still describe that file, and drop
/// them only when they no longer do.
///
/// Rows form an exact chain, so the newest surviving row's `text_after` must
/// equal the restored content. Clearing unconditionally made undoing a later
/// message erase Keep/Undo for earlier, still-valid edits.
///
/// Returns `true` when the rows were kept.
pub fn retain_path_if_consistent(
    conn: &DbConn,
    session_id: &str,
    path: &str,
    restored: Option<&str>,
) -> AppResult<bool> {
    let Some(restored) = restored else {
        clear_path(conn, session_id, path)?;
        return Ok(false);
    };
    let normalized = crate::data::paths::normalized_path_key(std::path::Path::new(path));
    #[cfg(windows)]
    const SQL: &str = "SELECT text_after FROM pending_diffs WHERE session_id = ?1
                       AND (path COLLATE NOCASE = ?2 OR path COLLATE NOCASE = ?3)
                       ORDER BY seq DESC LIMIT 1";
    #[cfg(not(windows))]
    const SQL: &str = "SELECT text_after FROM pending_diffs WHERE session_id = ?1
                       AND (path = ?2 OR path = ?3) ORDER BY seq DESC LIMIT 1";
    let last: Option<String> = conn
        .query_row(SQL, params![session_id, path, normalized], |r| r.get(0))
        .ok();
    match last {
        None => Ok(true),
        Some(text) if text.replace("\r\n", "\n") == restored.replace("\r\n", "\n") => Ok(true),
        Some(_) => {
            clear_path(conn, session_id, path)?;
            Ok(false)
        }
    }
}

pub fn clear_session(conn: &DbConn, session_id: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM pending_diffs WHERE session_id = ?1",
        params![session_id],
    )?;
    Ok(())
}

/// Clear pending review rows for paths that were rolled back via file snapshots.
pub fn clear_paths(conn: &DbConn, session_id: &str, paths: &[String]) -> AppResult<()> {
    for path in paths {
        clear_path(conn, session_id, path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::db::test_support::TempDb;

    const SID: &str = "session-1";

    fn workspace(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("atelier-diff-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    fn seed(conn: &DbConn, path: &str) {
        insert(
            conn,
            SID,
            path,
            "a",
            "b",
            "a",
            "b",
            Some("utf-8"),
            false,
            Some("u1"),
        )
        .unwrap()
        .expect("row stored");
    }

    #[test]
    fn clear_path_matches_an_unnormalized_spelling_of_the_stored_key() {
        let root = workspace("spelling");
        let db = TempDb::new("diff-spelling");
        let conn = db.conn();
        let file = root.join("notes.md");
        std::fs::write(&file, "x").unwrap();

        seed(&conn, &crate::data::paths::normalized_path_key(&file));
        // Same file, spelled with a redundant current-directory component.
        let detour = root.join(".").join("notes.md");
        clear_path(&conn, SID, &detour.to_string_lossy()).unwrap();

        assert!(list_for_session(&conn, SID).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Apply an Edit over whole lines `from..=to` of `lines` and record it.
    fn edit_lines(
        conn: &DbConn,
        path: &str,
        lines: &mut Vec<String>,
        from: usize,
        to: usize,
        tag: &str,
    ) -> Option<String> {
        let text_before = lines.join("\n");
        let before = lines[from..=to].join("\n");
        for (i, line) in lines.iter_mut().enumerate().take(to + 1).skip(from) {
            *line = format!("P{i} {tag}");
        }
        let after = lines[from..=to].join("\n");
        let text_after = lines.join("\n");
        insert_merging(
            conn,
            SID,
            path,
            &before,
            &after,
            &text_before,
            &text_after,
            Some("utf-8"),
            false,
            Some("u1"),
        )
        .unwrap()
    }

    fn doc() -> Vec<String> {
        (0..30).map(|i| format!("P{i} orig")).collect()
    }

    #[test]
    fn overlapping_edits_fold_into_one_hunk_spanning_the_union() {
        let db = TempDb::new("diff-merge-union");
        let conn = db.conn();
        let original = doc().join("\n");
        let mut lines = doc();

        edit_lines(&conn, "a.md", &mut lines, 0, 10, "one").unwrap();
        edit_lines(&conn, "a.md", &mut lines, 4, 12, "two").unwrap();
        let last = edit_lines(&conn, "a.md", &mut lines, 2, 10, "three").unwrap();

        let rows = list_for_path(&conn, SID, "a.md").unwrap();
        assert_eq!(rows.len(), 1, "three overlapping edits become one hunk");
        let row = &rows[0];
        assert_eq!(row.id, last);
        assert_eq!(row.text_before, original);
        assert_eq!(row.text_after, lines.join("\n"));
        // Union is P0..=P12: red = originals, green = latest text.
        assert_eq!(row.before_snippet.lines().count(), 13);
        assert!(row.before_snippet.starts_with("P0 orig"));
        assert!(row.before_snippet.ends_with("P12 orig"));
        let after: Vec<&str> = row.after_snippet.lines().collect();
        assert_eq!(after[0], "P0 one");
        assert_eq!(after[2], "P2 three");
        assert_eq!(after[10], "P10 three");
        assert_eq!(after[12], "P12 two");
    }

    #[test]
    fn disjoint_edit_between_overlapping_ones_keeps_an_exact_chain() {
        let db = TempDb::new("diff-merge-chain");
        let conn = db.conn();
        let original = doc().join("\n");
        let mut lines = doc();

        edit_lines(&conn, "a.md", &mut lines, 0, 3, "a").unwrap();
        edit_lines(&conn, "a.md", &mut lines, 20, 21, "far").unwrap();
        edit_lines(&conn, "a.md", &mut lines, 2, 5, "b").unwrap();

        let rows = list_for_path(&conn, SID, "a.md").unwrap();
        assert_eq!(rows.len(), 2);
        // Disjoint edit first, then the merged hunk.
        assert_eq!(rows[0].text_before, original);
        let mut only_far = doc();
        for i in 20..=21 {
            only_far[i] = format!("P{i} far");
        }
        assert_eq!(rows[0].text_after, only_far.join("\n"));
        assert_eq!(rows[1].text_before, rows[0].text_after);
        assert_eq!(rows[1].text_after, lines.join("\n"));
        assert!(rows[0].seq < rows[1].seq);

        // Undo of the merged hunk lands on the text with only the far edit.
        let revert = reject(&conn, SID, &rows[1].id).unwrap().unwrap();
        assert_eq!(revert.text, only_far.join("\n"));
        assert_eq!(list_for_path(&conn, SID, "a.md").unwrap().len(), 1);
    }

    #[test]
    fn rollback_keeps_earlier_rows_when_restored_text_matches() {
        let db = TempDb::new("diff-retain");
        let conn = db.conn();
        let mut lines = doc();
        let mut turn1_text = String::new();

        let t0 = lines.join("\n");
        lines[0] = "P0 first".to_string();
        turn1_text = {
            let _ = &turn1_text;
            lines.join("\n")
        };
        insert(
            &conn, SID, "a.md", "P0 orig", "P0 first", &t0, &turn1_text,
            Some("utf-8"), false, Some("turn-1"),
        )
        .unwrap();

        assert!(retain_path_if_consistent(&conn, SID, "a.md", Some(&turn1_text)).unwrap());
        assert_eq!(list_for_path(&conn, SID, "a.md").unwrap().len(), 1);

        assert!(!retain_path_if_consistent(&conn, SID, "a.md", Some("something else")).unwrap());
        assert!(list_for_path(&conn, SID, "a.md").unwrap().is_empty());
    }

    #[test]
    fn edits_on_different_lines_stay_separate() {
        let db = TempDb::new("diff-merge-separate");
        let conn = db.conn();
        let mut lines = doc();

        edit_lines(&conn, "a.md", &mut lines, 0, 2, "a").unwrap();
        edit_lines(&conn, "a.md", &mut lines, 10, 12, "b").unwrap();

        assert_eq!(list_for_path(&conn, SID, "a.md").unwrap().len(), 2);
    }

    #[test]
    fn edits_from_another_turn_are_not_folded() {
        let db = TempDb::new("diff-merge-turns");
        let conn = db.conn();
        let mut lines = doc();

        let text_before = lines.join("\n");
        lines[1] = "P1 first".to_string();
        let text_after = lines.join("\n");
        insert(
            &conn, SID, "a.md", "P1 orig", "P1 first", &text_before, &text_after,
            Some("utf-8"), false, Some("turn-1"),
        )
        .unwrap();
        edit_lines(&conn, "a.md", &mut lines, 0, 3, "later").unwrap();

        assert_eq!(list_for_path(&conn, SID, "a.md").unwrap().len(), 2);
    }

    #[cfg(windows)]
    #[test]
    fn clear_path_ignores_case_when_the_rolled_back_file_is_already_gone() {
        let root = workspace("case");
        let db = TempDb::new("diff-case");
        let conn = db.conn();
        let file = root.join("Chapter.md");
        std::fs::write(&file, "x").unwrap();
        seed(&conn, &crate::data::paths::normalized_path_key(&file));

        // A snapshot rollback deletes the file first, so the surviving path is
        // only case-accurate up to the parent directory.
        std::fs::remove_file(&file).unwrap();
        clear_path(&conn, SID, &root.join("chapter.md").to_string_lossy()).unwrap();

        assert!(list_for_session(&conn, SID).unwrap().is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
