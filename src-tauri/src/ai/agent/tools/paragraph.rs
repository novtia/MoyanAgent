//! Paragraph (line) splitting helpers for prose files.
//!
//! One line = one paragraph (`\n` separated). Empty lines are empty paragraphs.
//! Used by `Grep` (to report line positions) and `Read` (to count/slice lines).

pub const PARAGRAPH_SEP: &str = "\n";

/// Split file text into paragraphs (one line each).
pub fn split_paragraphs(text: &str) -> Vec<String> {
    text.split('\n').map(str::to_string).collect()
}

pub fn join_paragraphs(paragraphs: &[String]) -> String {
    paragraphs.join(PARAGRAPH_SEP)
}

pub fn paragraph_count(text: &str) -> usize {
    split_paragraphs(text).len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_one_paragraph() {
        let text = "a\nb\nc";
        assert_eq!(paragraph_count(text), 3);
        assert_eq!(split_paragraphs(text), vec!["a", "b", "c"]);
    }

    #[test]
    fn empty_line_is_its_own_paragraph() {
        let text = "a\n\nb";
        assert_eq!(paragraph_count(text), 3);
    }

    #[test]
    fn round_trip_join_split() {
        let parts = vec!["x".into(), String::new(), "y".into()];
        assert_eq!(split_paragraphs(&join_paragraphs(&parts)), parts);
    }
}
