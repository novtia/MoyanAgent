//! Paragraph splitting / numbering for prose files.
//!
//! One line = one paragraph (`\n` separated). Empty lines are empty paragraphs.

use serde_json::Value;

use crate::error::{AppError, AppResult};

pub const PARAGRAPH_SEP: &str = "\n";

/// An inclusive 1-based paragraph range resolved from the `Edit` `from` arg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParagraphRange {
    pub from: usize,
    pub to: usize,
}

/// Parse the `Edit` `from` argument into an inclusive contiguous range.
///
/// Accepts:
/// - an integer (`5`) or plain-number string (`"5"`) → single paragraph
/// - a range string (`"1-9"`, `"1~9"`, `"1..9"`) → `[1, 9]`
/// - a comma / ideographic-comma enumeration (`"1,2,3"`, `"1、2、3"`) →
///   sorted, deduped, and required to be contiguous (`max - min + 1 == len`).
///
/// Non-contiguous enumerations (e.g. `"1,3,5"`) are rejected.
pub fn parse_paragraph_spec(raw: Option<&Value>) -> AppResult<ParagraphRange> {
    let raw = raw.ok_or_else(|| AppError::Invalid("Edit: `from` is required".into()))?;

    if let Some(n) = raw.as_u64() {
        let n = n as usize;
        return make_range(n, n);
    }

    let s = raw
        .as_str()
        .ok_or_else(|| AppError::Invalid("Edit: `from` must be an integer or a string".into()))?
        .trim();
    if s.is_empty() {
        return Err(AppError::Invalid("Edit: `from` must not be empty".into()));
    }

    // Range syntax: "1-9", "1~9", "1..9".
    for sep in ["..", "~", "-"] {
        if let Some((a, b)) = s.split_once(sep) {
            let from = parse_index(a)?;
            let to = parse_index(b)?;
            return make_range(from, to);
        }
    }

    // Enumeration syntax: "1,2,3" or "1、2、3" (also tolerates "，").
    let parts: Vec<&str> = s
        .split(|c| c == ',' || c == '、' || c == '，')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        return Err(AppError::Invalid(format!("Edit: cannot parse `from`: {s}")));
    }
    if parts.len() == 1 {
        let n = parse_index(parts[0])?;
        return make_range(n, n);
    }

    let mut nums: Vec<usize> = parts
        .iter()
        .map(|p| parse_index(p))
        .collect::<AppResult<Vec<_>>>()?;
    nums.sort_unstable();
    nums.dedup();
    let from = nums[0];
    let to = nums[nums.len() - 1];
    if to - from + 1 != nums.len() {
        return Err(AppError::Invalid(format!(
            "Edit: `from` enumeration {s} must be contiguous — use a range like {from}-{to} instead"
        )));
    }
    make_range(from, to)
}

fn parse_index(s: &str) -> AppResult<usize> {
    s.trim()
        .parse::<usize>()
        .map_err(|_| AppError::Invalid(format!("Edit: invalid paragraph number `{}`", s.trim())))
}

fn make_range(from: usize, to: usize) -> AppResult<ParagraphRange> {
    if from == 0 {
        return Err(AppError::Invalid("Edit: paragraph numbers are 1-based".into()));
    }
    if to < from {
        return Err(AppError::Invalid(format!(
            "Edit: range end {to} must be >= start {from}"
        )));
    }
    Ok(ParagraphRange { from, to })
}

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

/// Prefix each line with `[P001]`, `[P002]`, … for agent Read output.
pub fn number_paragraphs(text: &str) -> String {
    number_paragraph_range(text, 1, paragraph_count(text))
}

/// Prefix paragraphs in the 1-based inclusive range `[from, to]`.
pub fn number_paragraph_range(text: &str, from: usize, to: usize) -> String {
    split_paragraphs(text)
        .into_iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let n = i + 1;
            if n >= from && n <= to {
                Some(format!("[P{:03}] {p}", n))
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(PARAGRAPH_SEP)
}

/// Split agent-supplied insert/replace text into lines, stripping
/// any `[Pnnn]` labels copied from Read output.
pub fn split_agent_paragraphs(text: &str) -> Vec<String> {
    split_paragraphs(text)
        .into_iter()
        .map(|p| strip_paragraph_label(&p).to_string())
        .collect()
}

/// Strip an optional `[P123]` prefix the model may copy from Read output.
pub fn strip_paragraph_label(s: &str) -> &str {
    let trimmed = s.trim_start();
    if !trimmed.starts_with("[P") {
        return s;
    }
    let Some(rest) = trimmed.strip_prefix("[P") else {
        return s;
    };
    let Some((digits, after)) = rest.split_once(']') else {
        return s;
    };
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return s;
    }
    after.trim_start()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_line_one_paragraph() {
        let text = "a\nb\nc";
        let numbered = number_paragraphs(text);
        assert!(numbered.contains("[P001] a"));
        assert!(numbered.contains("[P002] b"));
        assert!(numbered.contains("[P003] c"));
        assert_eq!(paragraph_count(text), 3);
    }

    #[test]
    fn empty_line_is_its_own_paragraph() {
        let text = "a\n\nb";
        assert_eq!(paragraph_count(text), 3);
        let numbered = number_paragraphs(text);
        assert!(numbered.contains("[P001] a"));
        assert!(numbered.contains("[P002] "));
        assert!(numbered.contains("[P003] b"));
    }

    #[test]
    fn round_trip_join_split() {
        let parts = vec!["x".into(), String::new(), "y".into()];
        assert_eq!(split_paragraphs(&join_paragraphs(&parts)), parts);
    }

    #[test]
    fn number_paragraph_range_middle_slice() {
        let text = "a\nb\nc\nd\ne";
        let numbered = number_paragraph_range(text, 2, 4);
        assert_eq!(numbered, "[P002] b\n[P003] c\n[P004] d");
    }

    #[test]
    fn number_paragraph_range_single() {
        let text = "a\nb\nc";
        assert_eq!(number_paragraph_range(text, 2, 2), "[P002] b");
    }

    #[test]
    fn number_paragraph_range_full_matches_whole_file() {
        let text = "a\n\nb";
        assert_eq!(number_paragraphs(text), number_paragraph_range(text, 1, 3));
    }

    use serde_json::json;

    fn spec(v: serde_json::Value) -> ParagraphRange {
        parse_paragraph_spec(Some(&v)).unwrap()
    }

    #[test]
    fn spec_integer_is_single_paragraph() {
        assert_eq!(spec(json!(5)), ParagraphRange { from: 5, to: 5 });
    }

    #[test]
    fn spec_numeric_string_is_single_paragraph() {
        assert_eq!(spec(json!("5")), ParagraphRange { from: 5, to: 5 });
    }

    #[test]
    fn spec_dash_range() {
        assert_eq!(spec(json!("1-9")), ParagraphRange { from: 1, to: 9 });
    }

    #[test]
    fn spec_tilde_and_dotdot_ranges() {
        assert_eq!(spec(json!("2~4")), ParagraphRange { from: 2, to: 4 });
        assert_eq!(spec(json!("2..4")), ParagraphRange { from: 2, to: 4 });
    }

    #[test]
    fn spec_contiguous_enumeration() {
        assert_eq!(spec(json!("1,2,3,4")), ParagraphRange { from: 1, to: 4 });
        assert_eq!(spec(json!("3、4、5")), ParagraphRange { from: 3, to: 5 });
    }

    #[test]
    fn spec_unsorted_but_contiguous_enumeration() {
        assert_eq!(spec(json!("3,1,2")), ParagraphRange { from: 1, to: 3 });
    }

    #[test]
    fn spec_rejects_non_contiguous_enumeration() {
        assert!(parse_paragraph_spec(Some(&json!("1,3,5"))).is_err());
    }

    #[test]
    fn spec_rejects_zero_and_missing() {
        assert!(parse_paragraph_spec(Some(&json!(0))).is_err());
        assert!(parse_paragraph_spec(None).is_err());
    }

    #[test]
    fn spec_rejects_reversed_range() {
        assert!(parse_paragraph_spec(Some(&json!("9-1"))).is_err());
    }
}
