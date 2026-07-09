//! Read-context expansion for ranged `Read` calls, plus the read-receipt
//! bookkeeping shared by the `Read`, `Write`, `Edit` and `CreateDoc` tools.
//!
//! When the model requests fewer than [`MIN_READ_CONTEXT_LINES`] paragraphs,
//! the Read tool silently expands the range to include surrounding context.
//!
//! A *receipt* records the content hash observed the last time the agent read
//! or wrote a file. `Edit`/`Write` refuse to touch a file with no receipt, and
//! `Edit` additionally rejects a stale receipt (the on-disk content no longer
//! matches what the model last saw), which is the main guard against
//! paragraph-number drift after out-of-band edits.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Minimum paragraphs returned by a ranged Read (system auto-expands).
pub const MIN_READ_CONTEXT_LINES: usize = 20;

/// Stable content hash used for read-receipt / stale-file detection.
///
/// Uses [`std::collections::hash_map::DefaultHasher`], which is seeded with
/// fixed keys and therefore deterministic within (and across) a process — good
/// enough for equality checks, never persisted.
pub fn content_hash(text: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Record (or refresh) a receipt: canonical path → hash of the text the model
/// now has in hand. Falls back to the raw path when canonicalization fails.
pub fn record_receipt(state: &Mutex<HashMap<PathBuf, u64>>, path: &Path, text: &str) {
    let key = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Ok(mut s) = state.lock() {
        s.insert(key, content_hash(text));
    }
}

/// Look up the stored content hash for `path`, matching either directly or via
/// canonicalization (receipts are keyed canonically, but callers may pass a
/// non-canonical path).
pub fn lookup_receipt(state: &Mutex<HashMap<PathBuf, u64>>, path: &Path) -> Option<u64> {
    let canonical = std::fs::canonicalize(path).ok();
    let s = state.lock().ok()?;
    for (p, hash) in s.iter() {
        if p == path {
            return Some(*hash);
        }
        if let (Some(a), Some(b)) = (canonical.as_ref(), std::fs::canonicalize(p).ok()) {
            if a == &b {
                return Some(*hash);
            }
        }
    }
    None
}

/// True iff the agent holds any receipt for `path` (read-first guard).
pub fn has_receipt(state: &Mutex<HashMap<PathBuf, u64>>, path: &Path) -> bool {
    lookup_receipt(state, path).is_some()
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
}
