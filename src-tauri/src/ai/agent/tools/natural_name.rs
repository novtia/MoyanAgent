//! Filename order that treats a whole number as one value.
//!
//! A plain code-point compare stops at the first different character, so
//! `第一章` sorts before `第三章` but after neither `第二章` nor `第十章`.
//! Digit runs and Chinese numerals are compared as integers; the rest of the
//! name is still compared in full.

use std::cmp::Ordering;
use std::path::Path;

#[derive(Debug)]
enum Tok<'a> {
    Text(&'a str),
    Num(u128),
}

/// Directories first, then natural filename order, then the full path.
pub fn cmp_listing(
    a_is_dir: bool,
    a_name: &str,
    a_path: &str,
    b_is_dir: bool,
    b_name: &str,
    b_path: &str,
) -> Ordering {
    b_is_dir
        .cmp(&a_is_dir)
        .then_with(|| cmp_natural_name(a_name, b_name))
        .then_with(|| cmp_text(a_path, b_path))
}

/// Natural order of two display names. Equal names compare equal; callers
/// that need a total order should follow this with the full path.
pub fn cmp_natural_name(a: &str, b: &str) -> Ordering {
    let ta = tokenize(a);
    let tb = tokenize(b);
    let n = ta.len().min(tb.len());
    for i in 0..n {
        let ord = match (&ta[i], &tb[i]) {
            (Tok::Num(x), Tok::Num(y)) => x.cmp(y),
            (Tok::Text(x), Tok::Text(y)) => cmp_text(x, y),
            (Tok::Num(_), Tok::Text(_)) => Ordering::Less,
            (Tok::Text(_), Tok::Num(_)) => Ordering::Greater,
        };
        if ord != Ordering::Equal {
            return ord;
        }
    }
    ta.len().cmp(&tb.len()).then_with(|| cmp_text(a, b))
}

/// Natural order of two paths, using each file's own name.
pub fn cmp_natural_path(a: &Path, b: &Path) -> Ordering {
    let a_name = a.file_name().map(|n| n.to_string_lossy());
    let b_name = b.file_name().map(|n| n.to_string_lossy());
    cmp_natural_name(
        a_name.as_deref().unwrap_or(""),
        b_name.as_deref().unwrap_or(""),
    )
    .then_with(|| a.cmp(b))
}

fn tokenize(s: &str) -> Vec<Tok<'_>> {
    let chars: Vec<(usize, char)> = s.char_indices().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let (start, ch) = chars[i];
        if ch.is_ascii_digit() {
            let mut j = i + 1;
            while j < chars.len() && chars[j].1.is_ascii_digit() {
                j += 1;
            }
            let end = chars.get(j).map(|c| c.0).unwrap_or(s.len());
            out.push(Tok::Num(parse_ascii(&s[start..end])));
            i = j;
        } else if is_cjk_numeral(ch) {
            let mut j = i + 1;
            while j < chars.len() && is_cjk_numeral(chars[j].1) {
                j += 1;
            }
            let end = chars.get(j).map(|c| c.0).unwrap_or(s.len());
            let raw = &s[start..end];
            match parse_chinese(raw) {
                Some(n) => out.push(Tok::Num(n)),
                None => out.push(Tok::Text(raw)),
            }
            i = j;
        } else {
            let mut j = i + 1;
            while j < chars.len() && !chars[j].1.is_ascii_digit() && !is_cjk_numeral(chars[j].1) {
                j += 1;
            }
            let end = chars.get(j).map(|c| c.0).unwrap_or(s.len());
            out.push(Tok::Text(&s[start..end]));
            i = j;
        }
    }
    out
}

fn cmp_text(a: &str, b: &str) -> Ordering {
    let mut ia = a.chars();
    let mut ib = b.chars();
    loop {
        match (ia.next(), ib.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let x = if x.is_ascii() {
                    x.to_ascii_lowercase()
                } else {
                    x
                };
                let y = if y.is_ascii() {
                    y.to_ascii_lowercase()
                } else {
                    y
                };
                let ord = x.cmp(&y);
                if ord != Ordering::Equal {
                    return ord;
                }
            }
        }
    }
}

fn parse_ascii(raw: &str) -> u128 {
    let mut n: u128 = 0;
    for c in raw.chars() {
        let Some(d) = c.to_digit(10) else {
            return n;
        };
        n = n.saturating_mul(10).saturating_add(u128::from(d));
    }
    n
}

fn is_cjk_numeral(c: char) -> bool {
    chinese_digit(c).is_some() || chinese_unit(c).is_some()
}

fn chinese_digit(c: char) -> Option<u32> {
    Some(match c {
        '零' | '〇' => 0,
        '一' | '壹' => 1,
        '二' | '贰' | '貳' | '两' | '兩' => 2,
        '三' | '叁' | '參' => 3,
        '四' | '肆' => 4,
        '五' | '伍' => 5,
        '六' | '陆' | '陸' => 6,
        '七' | '柒' => 7,
        '八' | '捌' => 8,
        '九' | '玖' => 9,
        _ => return None,
    })
}

fn chinese_unit(c: char) -> Option<u128> {
    Some(match c {
        '十' | '拾' => 10,
        '百' | '佰' => 100,
        '千' | '仟' => 1000,
        '万' | '萬' => 10_000,
        _ => return None,
    })
}

/// Place-value Chinese numerals (`十一`, `二十`, `一百零一`).
/// A run of digits with no unit is read positionally (`一二三` = 123).
fn parse_chinese(raw: &str) -> Option<u128> {
    let chars: Vec<char> = raw.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let has_unit = chars.iter().any(|c| chinese_unit(*c).is_some());
    if !has_unit {
        let mut n: u128 = 0;
        for c in chars {
            let d = chinese_digit(c)?;
            n = n.checked_mul(10)?.checked_add(u128::from(d))?;
        }
        return Some(n);
    }

    let mut total: u128 = 0;
    let mut section: u128 = 0;
    let mut number: u128 = 0;
    for c in chars {
        if let Some(d) = chinese_digit(c) {
            number = number.checked_mul(10)?.checked_add(u128::from(d))?;
        } else if let Some(unit) = chinese_unit(c) {
            if unit == 10_000 {
                if number == 0 && section == 0 {
                    number = 1;
                }
                section = section.checked_add(number)?;
                total = total.checked_add(section.checked_mul(10_000)?)?;
                section = 0;
                number = 0;
            } else {
                if number == 0 {
                    number = 1;
                }
                section = section.checked_add(number.checked_mul(unit)?)?;
                number = 0;
            }
        } else {
            return None;
        }
    }
    total.checked_add(section)?.checked_add(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut owned: Vec<String> = names.iter().map(|s| (*s).to_string()).collect();
        owned.sort_by(|a, b| cmp_natural_name(a, b));
        owned
    }

    #[test]
    fn chapters_follow_the_whole_number() {
        assert_eq!(
            sorted(&[
                "第三章",
                "第一章",
                "第十章",
                "第二章",
                "第十一章",
                "第二十章"
            ]),
            vec![
                "第一章",
                "第二章",
                "第三章",
                "第十章",
                "第十一章",
                "第二十章",
            ]
        );
    }

    #[test]
    fn extras_follow_the_whole_number() {
        assert_eq!(
            sorted(&["番外三", "番外一", "番外十", "番外十一", "番外二", "番外四"]),
            vec!["番外一", "番外二", "番外三", "番外四", "番外十", "番外十一"]
        );
    }

    #[test]
    fn arabic_numbers_are_not_compared_digit_by_digit() {
        assert_eq!(cmp_natural_name("第2章", "第10章"), Ordering::Less);
        assert_eq!(
            cmp_natural_name("第10章.txt", "第2章.txt"),
            Ordering::Greater
        );
    }

    #[test]
    fn longer_text_is_compared_past_the_shared_prefix() {
        assert_eq!(cmp_natural_name("后续", "后续要求"), Ordering::Less);
        assert_eq!(cmp_natural_name("后续要求", "后续"), Ordering::Greater);
    }

    #[test]
    fn large_chinese_numerals_keep_their_value() {
        assert_eq!(
            cmp_natural_name("第九十九章", "第一百零一章"),
            Ordering::Less
        );
    }

    #[test]
    fn directories_stay_before_files() {
        assert_eq!(
            cmp_listing(true, "第二章", "b", false, "第一章", "a"),
            Ordering::Less
        );
    }
}
