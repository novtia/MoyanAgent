//! Paragraph splitting / numbering for prose files.
//!
//! One line = one paragraph (`\n` separated). Empty lines are empty paragraphs.

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

/// Insert one or more lines immediately after `after_index`.
pub fn insert_paragraphs_after(
    paragraphs: &mut Vec<String>,
    after_index: usize,
    content: &str,
) -> usize {
    let new_paras = split_agent_paragraphs(content);
    if new_paras.is_empty() || new_paras.iter().all(|p| p.is_empty()) {
        return 0;
    }
    let insert_at = after_index + 1;
    for (offset, p) in new_paras.iter().enumerate() {
        paragraphs.insert(insert_at + offset, p.clone());
    }
    new_paras.len()
}

/// Replace paragraphs in the inclusive 1-based range `[from, to]` with lines from `content`.
pub fn replace_paragraph_range(
    paragraphs: &mut Vec<String>,
    from: usize,
    to: usize,
    content: &str,
) {
    let from_idx = from.saturating_sub(1);
    let to_idx = to.saturating_sub(1);
    if from_idx > to_idx || to_idx >= paragraphs.len() {
        return;
    }
    let new_paras = split_agent_paragraphs(content);
    paragraphs.splice(from_idx..=to_idx, new_paras);
}

/// Replace paragraph at `index` with one or more lines from `content`.
pub fn replace_paragraph_with(paragraphs: &mut Vec<String>, index: usize, content: &str) {
    let new_paras = split_agent_paragraphs(content);
    if new_paras.is_empty() {
        paragraphs[index] = String::new();
        return;
    }
    if new_paras.len() == 1 {
        paragraphs[index] = new_paras[0].clone();
        return;
    }
    paragraphs.splice(index..=index, new_paras);
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
    fn insert_after_paragraph() {
        let mut paras = vec![
            "哈哈哈哈哈".into(),
            "呜呜呜呜".into(),
            "哦哦哦".into(),
        ];
        let n = insert_paragraphs_after(&mut paras, 1, "哈咦咦咦咦\n嚯嚯嚯\n歪歪");
        assert_eq!(n, 3);
        assert_eq!(
            paras,
            vec![
                "哈哈哈哈哈",
                "呜呜呜呜",
                "哈咦咦咦咦",
                "嚯嚯嚯",
                "歪歪",
                "哦哦哦",
            ]
        );
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
    fn replace_paragraph_range_multi() {
        let mut paras = vec!["a".into(), "b".into(), "c".into(), "d".into(), "e".into()];
        replace_paragraph_range(&mut paras, 2, 4, "X\nY");
        assert_eq!(paras, vec!["a", "X", "Y", "e"]);
    }

    #[test]
    fn number_paragraph_range_full_matches_whole_file() {
        let text = "a\n\nb";
        assert_eq!(number_paragraphs(text), number_paragraph_range(text, 1, 3));
    }
}
