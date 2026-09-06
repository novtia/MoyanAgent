//! Read-context expansion for ranged `Read` calls, plus the read-receipt
//! bookkeeping shared by the `Read`, `Write`, `Edit` and `CreateDoc` tools.
//!
//! When the model requests fewer than [`MIN_READ_CONTEXT_LINES`] paragraphs,
//! the Read tool silently expands the range to include surrounding context.
//!
//! A *receipt* records the content hash observed the last time the agent read
//! or wrote a file. A write-origin receipt lets Read return a stub instead of
//! echoing back a body the model already submitted in CreateDoc / Write / Edit.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::ai::agent::core::context::FileReceipt;

/// Minimum paragraphs returned by a ranged Read (system auto-expands).
pub const MIN_READ_CONTEXT_LINES: usize = 20;

/// Shown on CreateDoc / Write success and on a short-circuited Read of a
/// file the model just wrote. Tells the model not to pull the body back in.
pub const DO_NOT_REREAD_NOTE: &str = "Do not Read this file back — the body is already \
in this turn's CreateDoc/Write/Edit arguments. Copy a short unique Edit `old_string` \
from the insertion point or tail of that text, never the whole chapter. \
Read again only after Edit fails.";

/// Shown when Read is asked for a span it already returned this turn.
pub const ALREADY_READ_NOTE: &str = "This span was already returned by an earlier Read \
this turn. Copy Edit `old_string` from that Read output. Do not Read the same \
paragraphs again.";

/// Stable content hash used for read-receipt equality checks.
///
/// Uses [`std::collections::hash_map::DefaultHasher`], which is seeded with
/// fixed keys and therefore deterministic within (and across) a process — good
/// enough for equality checks, never persisted.
pub fn content_hash(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

fn receipt_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Record (or refresh) a receipt: canonical path → hash of the text the model
/// now has in hand. Falls back to the raw path when canonicalization fails.
///
/// `from_write` is true after CreateDoc / Write / a successful Edit, so a later
/// Read of the same bytes can skip echoing the body.
pub fn record_receipt(
    state: &Mutex<HashMap<PathBuf, FileReceipt>>,
    path: &Path,
    text: &str,
    from_write: bool,
) {
    let key = receipt_key(path);
    if let Ok(mut s) = state.lock() {
        s.insert(
            key,
            FileReceipt {
                hash: content_hash(text),
                from_write,
                covered: Vec::new(),
            },
        );
    }
}

/// Drop the receipt so the next Read returns the on-disk body (Edit missed).
pub fn clear_receipt(state: &Mutex<HashMap<PathBuf, FileReceipt>>, path: &Path) {
    let key = receipt_key(path);
    if let Ok(mut s) = state.lock() {
        s.remove(&key);
    }
}

/// True when this path was last written by the agent and `text` still matches.
pub fn is_fresh_write(
    state: &Mutex<HashMap<PathBuf, FileReceipt>>,
    path: &Path,
    text: &str,
) -> bool {
    let key = receipt_key(path);
    let Ok(s) = state.lock() else {
        return false;
    };
    s.get(&key)
        .is_some_and(|r| r.from_write && r.hash == content_hash(text))
}

/// Record that a Read already returned `[from, to]` (inclusive, 1-based).
///
/// If the file bytes changed, the previous spans are dropped.
pub fn record_read_span(
    state: &Mutex<HashMap<PathBuf, FileReceipt>>,
    path: &Path,
    text: &str,
    from: usize,
    to: usize,
) {
    let key = receipt_key(path);
    let hash = content_hash(text);
    let (from, to) = (from.min(to), from.max(to));
    if let Ok(mut s) = state.lock() {
        match s.get_mut(&key) {
            Some(r) if r.hash == hash => {
                r.covered = union_range(&r.covered, from, to);
            }
            _ => {
                s.insert(
                    key,
                    FileReceipt {
                        hash,
                        from_write: false,
                        covered: vec![(from, to)],
                    },
                );
            }
        }
    }
}

/// When the file is unchanged and `from..=to` sits inside a span already
/// returned this turn, the covered ranges. `None` means Read should return
/// the body.
pub fn already_read_span(
    state: &Mutex<HashMap<PathBuf, FileReceipt>>,
    path: &Path,
    text: &str,
    from: usize,
    to: usize,
) -> Option<Vec<(usize, usize)>> {
    let key = receipt_key(path);
    let hash = content_hash(text);
    let (from, to) = (from.min(to), from.max(to));
    let Ok(s) = state.lock() else {
        return None;
    };
    s.get(&key).and_then(|r| {
        if r.hash == hash && range_covered(&r.covered, from, to) {
            Some(r.covered.clone())
        } else {
            None
        }
    })
}

fn union_range(ranges: &[(usize, usize)], from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut v: Vec<(usize, usize)> = ranges.to_vec();
    v.push((from, to));
    v.sort_by_key(|r| r.0);
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (a, b) in v {
        if let Some(last) = out.last_mut() {
            if a <= last.1.saturating_add(1) {
                last.1 = last.1.max(b);
                continue;
            }
        }
        out.push((a, b));
    }
    out
}

fn range_covered(ranges: &[(usize, usize)], from: usize, to: usize) -> bool {
    ranges.iter().any(|&(a, b)| from >= a && to <= b)
}

/// Expand a requested inclusive 1-based range to at least
/// [`MIN_READ_CONTEXT_LINES`] paragraphs, centered on the request.
pub fn expand_read_range(from: usize, to: usize, file_total: usize) -> (usize, usize) {
    if file_total == 0 {
        return (1, 1);
    }
    if file_total <= MIN_READ_CONTEXT_LINES {
        return (1, file_total);
    }
    let requested_span = to.saturating_sub(from).saturating_add(1);
    if requested_span >= MIN_READ_CONTEXT_LINES {
        return (from, to);
    }
    let center = from + (requested_span - 1) / 2;
    let half = MIN_READ_CONTEXT_LINES / 2;
    let mut expanded_from = center.saturating_sub(half).max(1);
    let expanded_to = expanded_from
        .saturating_add(MIN_READ_CONTEXT_LINES - 1)
        .min(file_total);
    expanded_from = expanded_to
        .saturating_sub(MIN_READ_CONTEXT_LINES - 1)
        .max(1);
    (expanded_from, expanded_to)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[test]
    fn single_paragraph_expands_to_twenty() {
        let (from, to) = expand_read_range(50, 50, 200);
        assert_eq!(to - from + 1, MIN_READ_CONTEXT_LINES);
        assert!(from <= 50 && 50 <= to);
    }

    #[test]
    fn short_range_expands_centered() {
        let (from, to) = expand_read_range(48, 52, 200);
        assert_eq!(to - from + 1, MIN_READ_CONTEXT_LINES);
        assert!(from <= 50 && 50 <= to);
    }

    #[test]
    fn already_wide_range_unchanged() {
        assert_eq!(expand_read_range(10, 35, 100), (10, 35));
    }

    #[test]
    fn small_file_returns_whole_file() {
        assert_eq!(expand_read_range(2, 3, 8), (1, 8));
    }

    #[test]
    fn range_near_file_start_clamps() {
        let (from, to) = expand_read_range(1, 1, 100);
        assert_eq!(from, 1);
        assert_eq!(to - from + 1, MIN_READ_CONTEXT_LINES);
    }

    #[test]
    fn range_near_file_end_clamps() {
        let (from, to) = expand_read_range(98, 100, 100);
        assert_eq!(to, 100);
        assert_eq!(to - from + 1, MIN_READ_CONTEXT_LINES);
    }

    #[test]
    fn write_receipt_matches_until_cleared() {
        let dir = std::env::temp_dir().join(format!("moyan-receipt-{}-{}", std::process::id(), 1));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ch.txt");
        std::fs::write(&path, "hello").unwrap();
        let state = Mutex::new(HashMap::new());
        record_receipt(&state, &path, "hello", true);
        assert!(is_fresh_write(&state, &path, "hello"));
        assert!(!is_fresh_write(&state, &path, "other"));
        clear_receipt(&state, &path);
        assert!(!is_fresh_write(&state, &path, "hello"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn overlapping_read_span_is_already_read() {
        let dir = std::env::temp_dir().join(format!("moyan-receipt-{}-{}", std::process::id(), 2));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ch.txt");
        std::fs::write(&path, "hello").unwrap();
        let state = Mutex::new(HashMap::new());
        record_read_span(&state, &path, "hello", 100, 130);
        assert!(already_read_span(&state, &path, "hello", 110, 116).is_some());
        assert!(already_read_span(&state, &path, "hello", 100, 130).is_some());
        assert!(already_read_span(&state, &path, "hello", 1, 40).is_none());
        assert!(already_read_span(&state, &path, "hello", 125, 140).is_none());
        assert!(already_read_span(&state, &path, "changed", 110, 116).is_none());
        record_read_span(&state, &path, "hello", 131, 140);
        assert!(already_read_span(&state, &path, "hello", 120, 135).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
