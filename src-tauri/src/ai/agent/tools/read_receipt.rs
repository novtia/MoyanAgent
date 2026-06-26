//! Read-context expansion for ranged `Read` calls.
//!
//! When the model requests fewer than [`MIN_READ_CONTEXT_LINES`] paragraphs,
//! the Read tool silently expands the range to include surrounding context.

/// Minimum paragraphs returned by a ranged Read (system auto-expands).
pub const MIN_READ_CONTEXT_LINES: usize = 20;

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
