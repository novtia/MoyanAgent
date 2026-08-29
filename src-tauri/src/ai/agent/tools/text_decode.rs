//! Decode / encode text files across common on-disk encodings.
//!
//! Windows `.txt` files are often UTF-16 LE (Notepad), GBK/CP936, or Big5
//! without a BOM — not UTF-8. [`detect_and_decode`] picks an encoding; callers
//! should write back with the same encoding via [`write_text_file`].

use std::path::Path;

use chardetng::EncodingDetector;
use encoding_rs::Encoding;
use serde::{Deserialize, Serialize};

/// Supported on-disk text encodings (labels are stable API surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextEncoding {
    Utf8,
    Utf16Le,
    Utf16Be,
    Gbk,
    Big5,
    ShiftJis,
    EucKr,
    Windows1252,
}

impl TextEncoding {
    pub fn label(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf16Le => "utf-16le",
            Self::Utf16Be => "utf-16be",
            Self::Gbk => "gbk",
            Self::Big5 => "big5",
            Self::ShiftJis => "shift-jis",
            Self::EucKr => "euc-kr",
            Self::Windows1252 => "windows-1252",
        }
    }

    pub fn parse_label(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "utf-8" | "utf8" => Self::Utf8,
            "utf-16le" | "utf16le" | "utf-16-le" => Self::Utf16Le,
            "utf-16be" | "utf16be" | "utf-16-be" => Self::Utf16Be,
            "gbk" | "gb2312" | "gb18030" | "cp936" | "windows-936" => Self::Gbk,
            "big5" | "cp950" | "windows-950" => Self::Big5,
            "shift-jis" | "shift_jis" | "sjis" | "cp932" | "windows-932" => Self::ShiftJis,
            "euc-kr" | "euc_kr" | "cp949" | "windows-949" => Self::EucKr,
            "windows-1252" | "cp1252" | "latin1" | "iso-8859-1" | "iso8859-1" => {
                Self::Windows1252
            }
            _ => Self::Utf8,
        }
    }

    fn encoding(self) -> &'static Encoding {
        match self {
            Self::Utf8 => encoding_rs::UTF_8,
            Self::Utf16Le => encoding_rs::UTF_16LE,
            Self::Utf16Be => encoding_rs::UTF_16BE,
            Self::Gbk => encoding_rs::GBK,
            Self::Big5 => encoding_rs::BIG5,
            Self::ShiftJis => encoding_rs::SHIFT_JIS,
            Self::EucKr => encoding_rs::EUC_KR,
            Self::Windows1252 => encoding_rs::WINDOWS_1252,
        }
    }

    fn from_encoding_rs(enc: &'static Encoding) -> Self {
        if enc == encoding_rs::UTF_8 {
            Self::Utf8
        } else if enc == encoding_rs::UTF_16LE {
            Self::Utf16Le
        } else if enc == encoding_rs::UTF_16BE {
            Self::Utf16Be
        } else if enc == encoding_rs::GBK || enc == encoding_rs::GB18030 {
            Self::Gbk
        } else if enc == encoding_rs::BIG5 {
            Self::Big5
        } else if enc == encoding_rs::SHIFT_JIS {
            Self::ShiftJis
        } else if enc == encoding_rs::EUC_KR {
            Self::EucKr
        } else if enc == encoding_rs::WINDOWS_1252 || enc == encoding_rs::ISO_8859_15 {
            Self::Windows1252
        } else {
            Self::Utf8
        }
    }
}

#[derive(Debug, Clone)]
pub struct DecodedText {
    pub text: String,
    pub encoding: TextEncoding,
    pub had_bom: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectTextFile {
    pub text: String,
    pub encoding: String,
    pub had_bom: bool,
}

impl From<DecodedText> for ProjectTextFile {
    fn from(value: DecodedText) -> Self {
        Self {
            text: value.text,
            encoding: value.encoding.label().to_string(),
            had_bom: value.had_bom,
        }
    }
}

/// Normalize a **prose** tool argument string after JSON parsing.
///
/// Models and partial JSON repair sometimes leave JSON-style escapes (`\"`, `\n`, …)
/// in prose `content` fields. Run until stable so nested escapes like `\\"` are
/// fully resolved before writing to disk.
///
/// Only for prose destinations — `CreateDoc` content and `Edit`'s not-found
/// retry. Never apply it to code-bearing writes: source text legitimately
/// contains `\n`, `\\` and `\"`, and collapsing those corrupts the file.
pub fn normalize_tool_string(s: &str) -> String {
    let mut cur = s.to_string();
    for _ in 0..8 {
        let next = normalize_tool_string_once(&cur);
        if next == cur {
            break;
        }
        cur = next;
    }
    // Final sweep: stray `\"` pairs still seen in streamed CreateDoc / Edit payloads.
    while cur.contains("\\\"") {
        cur = cur.replace("\\\"", "\"");
    }
    // HTML entities occasionally leak from model output or partial JSON repair.
    cur = decode_html_entities(&cur);
    cur
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#34;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn normalize_tool_string_once(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.peek().copied() {
                Some('"') => {
                    it.next();
                    out.push('"');
                }
                Some('\\') => {
                    it.next();
                    out.push('\\');
                }
                Some('n') => {
                    it.next();
                    out.push('\n');
                }
                Some('t') => {
                    it.next();
                    out.push('\t');
                }
                Some('r') => {
                    it.next();
                    out.push('\r');
                }
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// ASCII `"` plus typographic / CJK doubles (`“”「」『』…`).
pub fn is_double_quote(c: char) -> bool {
    matches!(
        c,
        '"' | '\u{201C}'
            | '\u{201D}'
            | '\u{201E}'
            | '\u{201F}'
            | '\u{00AB}'
            | '\u{00BB}'
            | '\u{300C}'
            | '\u{300D}'
            | '\u{300E}'
            | '\u{300F}'
            | '\u{FF02}'
    )
}

/// ASCII `'` plus typographic singles (`‘’`).
pub fn is_single_quote(c: char) -> bool {
    matches!(
        c,
        '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' | '\u{FF07}'
    )
}

pub fn is_quote_char(c: char) -> bool {
    is_double_quote(c) || is_single_quote(c)
}

/// Fold quote glyphs to ASCII so `“你好”` / `「你好」` / `"你好"` compare equal.
pub fn fold_quote_char(c: char) -> char {
    if is_double_quote(c) {
        '"'
    } else if is_single_quote(c) {
        '\''
    } else {
        c
    }
}

#[derive(Clone, Copy)]
enum QuoteSide {
    Opening,
    Closing,
    Neutral,
}

fn double_quote_side(c: char) -> QuoteSide {
    match c {
        '\u{201C}' | '\u{00AB}' | '\u{300C}' | '\u{300E}' | '\u{201F}' => QuoteSide::Opening,
        '\u{201D}' | '\u{00BB}' | '\u{300D}' | '\u{300F}' => QuoteSide::Closing,
        _ => QuoteSide::Neutral,
    }
}

fn single_quote_side(c: char) -> QuoteSide {
    match c {
        '\u{2018}' | '\u{201B}' => QuoteSide::Opening,
        '\u{2019}' => QuoteSide::Closing,
        _ => QuoteSide::Neutral,
    }
}

#[derive(Clone, Copy)]
struct PairStyle {
    open: char,
    close: char,
}

fn detect_quote_style(
    sample: &str,
    is_q: fn(char) -> bool,
    side: fn(char) -> QuoteSide,
) -> Option<PairStyle> {
    let mut open = None;
    let mut close = None;
    let mut neutral = None;
    for c in sample.chars() {
        if !is_q(c) {
            continue;
        }
        match side(c) {
            QuoteSide::Opening => {
                if open.is_none() {
                    open = Some(c);
                }
            }
            QuoteSide::Closing => {
                if close.is_none() {
                    close = Some(c);
                }
            }
            QuoteSide::Neutral => {
                if neutral.is_none() {
                    neutral = Some(c);
                }
            }
        }
    }
    match (open, close, neutral) {
        (Some(o), Some(c), _) => Some(PairStyle { open: o, close: c }),
        (Some(o), None, Some(n)) => Some(PairStyle { open: o, close: n }),
        (None, Some(c), Some(n)) => Some(PairStyle { open: n, close: c }),
        (Some(o), None, None) => Some(PairStyle { open: o, close: o }),
        (None, Some(c), None) => Some(PairStyle { open: c, close: c }),
        (None, None, Some(n)) => Some(PairStyle { open: n, close: n }),
        (None, None, None) => None,
    }
}

fn apply_quote_style(style: PairStyle, side: QuoteSide, open_next: &mut bool) -> char {
    if style.open == style.close {
        return style.open;
    }
    match side {
        QuoteSide::Opening => {
            *open_next = false;
            style.open
        }
        QuoteSide::Closing => {
            *open_next = true;
            style.close
        }
        QuoteSide::Neutral => {
            if *open_next {
                *open_next = false;
                style.open
            } else {
                *open_next = true;
                style.close
            }
        }
    }
}

/// Rewrite quote glyphs in `text` to match the style used in `sample`.
///
/// Used when Edit located `old_string` via quote-folding: the file keeps its
/// existing `"` / `“”` / `「」`, and `new_string` is rewritten to the same glyphs
/// so a curly-quote model call does not pollute an ASCII-quote chapter.
pub fn remap_quotes_to_sample(text: &str, sample: &str) -> String {
    let double = detect_quote_style(sample, is_double_quote, double_quote_side);
    let single = detect_quote_style(sample, is_single_quote, single_quote_side);
    let mut d_open_next = true;
    let mut s_open_next = true;
    text.chars()
        .map(|c| {
            if is_double_quote(c) {
                match double {
                    Some(style) => {
                        apply_quote_style(style, double_quote_side(c), &mut d_open_next)
                    }
                    None => c,
                }
            } else if is_single_quote(c) {
                match single {
                    Some(style) => {
                        apply_quote_style(style, single_quote_side(c), &mut s_open_next)
                    }
                    None => c,
                }
            } else {
                c
            }
        })
        .collect()
}

/// Non-overlapping byte ranges in `haystack` whose quote-folded text equals `needle`.
///
/// Empty when `needle` contains no quote glyphs — in that case folding cannot
/// find a match that exact search missed.
pub fn find_quote_folded_ranges(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    if needle.is_empty() || !needle.chars().any(is_quote_char) {
        return Vec::new();
    }
    let needle_folded: Vec<char> = needle.chars().map(fold_quote_char).collect();
    let n = needle_folded.len();
    let hay: Vec<(usize, char)> = haystack
        .char_indices()
        .map(|(i, c)| (i, fold_quote_char(c)))
        .collect();
    let hay_len = hay.len();
    if n == 0 || hay_len < n {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut i = 0;
    while i + n <= hay_len {
        let matched = hay[i..i + n]
            .iter()
            .zip(needle_folded.iter())
            .all(|((_, fc), nc)| fc == nc);
        if matched {
            let start = hay[i].0;
            let end = if i + n < hay_len {
                hay[i + n].0
            } else {
                haystack.len()
            };
            out.push((start, end));
            i += n;
        } else {
            i += 1;
        }
    }
    out
}

/// Decode file bytes for text consumers (returns Unicode text only).
pub fn decode_file_bytes(bytes: &[u8]) -> String {
    detect_and_decode(bytes).text
}

/// Detect encoding and decode to Unicode.
pub fn detect_and_decode(bytes: &[u8]) -> DecodedText {
    if bytes.is_empty() {
        return DecodedText {
            text: String::new(),
            encoding: TextEncoding::Utf8,
            had_bom: false,
        };
    }

    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        let text = encoding_rs::UTF_8
            .decode(&bytes[3..])
            .0
            .into_owned();
        return DecodedText {
            text,
            encoding: TextEncoding::Utf8,
            had_bom: true,
        };
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        if let Some(text) = decode_utf16_le(&bytes[2..]) {
            return DecodedText {
                text,
                encoding: TextEncoding::Utf16Le,
                had_bom: true,
            };
        }
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        if let Some(text) = decode_utf16_be(&bytes[2..]) {
            return DecodedText {
                text,
                encoding: TextEncoding::Utf16Be,
                had_bom: true,
            };
        }
    }

    if std::str::from_utf8(bytes).is_ok() {
        return DecodedText {
            text: String::from_utf8_lossy(bytes).into_owned(),
            encoding: TextEncoding::Utf8,
            had_bom: false,
        };
    }

    let mut detector = EncodingDetector::new();
    detector.feed(bytes, true);
    let enc = detector.guess(None, true);
    let encoding = TextEncoding::from_encoding_rs(enc);
    let text = enc.decode(bytes).0.into_owned();
    DecodedText {
        text,
        encoding,
        had_bom: false,
    }
}

pub fn encode_text(text: &str, encoding: TextEncoding, had_bom: bool) -> Vec<u8> {
    let (body, _, _) = encoding.encoding().encode(text);
    let mut out = Vec::with_capacity(body.len() + 3);
    if had_bom {
        match encoding {
            TextEncoding::Utf8 => out.extend_from_slice(&[0xEF, 0xBB, 0xBF]),
            TextEncoding::Utf16Le => out.extend_from_slice(&[0xFF, 0xFE]),
            TextEncoding::Utf16Be => out.extend_from_slice(&[0xFE, 0xFF]),
            _ => {}
        }
    }
    out.extend_from_slice(&body);
    out
}

pub fn read_text_file(path: &Path) -> std::io::Result<DecodedText> {
    let bytes = std::fs::read(path)?;
    Ok(detect_and_decode(&bytes))
}

pub fn write_text_file(
    path: &Path,
    text: &str,
    encoding: TextEncoding,
    had_bom: bool,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, encode_text(text, encoding, had_bom))
}

pub fn write_text_file_labeled(
    path: &Path,
    text: &str,
    encoding_label: Option<&str>,
    had_bom: Option<bool>,
) -> std::io::Result<()> {
    let encoding = encoding_label
        .map(TextEncoding::parse_label)
        .unwrap_or(TextEncoding::Utf8);
    let had_bom = had_bom.unwrap_or(false);
    write_text_file(path, text, encoding, had_bom)
}

fn decode_utf16_le(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let s = String::from_utf16_lossy(&units);
    if too_many_replacement_chars(&s) {
        return None;
    }
    Some(s)
}

fn decode_utf16_be(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    let s = String::from_utf16_lossy(&units);
    if too_many_replacement_chars(&s) {
        return None;
    }
    Some(s)
}

fn too_many_replacement_chars(s: &str) -> bool {
    let n = s.chars().count();
    if n == 0 {
        return false;
    }
    let bad = s.chars().filter(|&c| c == '\u{FFFD}').count();
    bad * 10 > n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_unescapes_literal_backslash_quote() {
        let input = "问题\\\"你现在还觉得";
        assert_eq!(normalize_tool_string(input), "问题\"你现在还觉得");
    }

    #[test]
    fn normalize_unescapes_dialogue_quotes() {
        let input = "\\\"孩子，你、你再试一次，魂力测试！\\\"他从怀里掏出一块白色的水晶球";
        assert_eq!(
            normalize_tool_string(input),
            "\"孩子，你、你再试一次，魂力测试！\"他从怀里掏出一块白色的水晶球"
        );
    }

    #[test]
    fn normalize_decodes_html_quot_entities() {
        let input = "&quot;我七岁呀，&quot;小舞扳着手指头算";
        assert_eq!(
            normalize_tool_string(input),
            "\"我七岁呀，\"小舞扳着手指头算"
        );
    }

    #[test]
    fn normalize_resolves_double_escaped_quotes() {
        let input = "\\\\\\\"hello\\\\\\\"";
        assert_eq!(normalize_tool_string(input), "\"hello\"");
    }

    #[test]
    fn quote_fold_finds_curly_needle_in_ascii_haystack() {
        let file = "他说\"你好\"。";
        let needle = "他说\u{201C}你好\u{201D}。";
        let ranges = find_quote_folded_ranges(file, needle);
        assert_eq!(ranges, vec![(0, file.len())]);
    }

    #[test]
    fn quote_fold_finds_ascii_needle_in_corner_brackets() {
        let file = "他说「你好」。";
        let needle = "他说\"你好\"。";
        let ranges = find_quote_folded_ranges(file, needle);
        assert_eq!(ranges, vec![(0, file.len())]);
    }

    #[test]
    fn quote_fold_skips_needles_without_quotes() {
        assert!(find_quote_folded_ranges("hello", "hello").is_empty());
    }

    #[test]
    fn remap_curly_new_string_to_ascii_file_span() {
        let out = remap_quotes_to_sample("他说\u{201C}再见\u{201D}。", "他说\"你好\"。");
        assert_eq!(out, "他说\"再见\"。");
    }

    #[test]
    fn remap_ascii_new_string_to_corner_brackets() {
        let out = remap_quotes_to_sample("他说\"再见\"。", "他说「你好」。");
        assert_eq!(out, "他说「再见」。");
    }

    #[test]
    fn utf8_roundtrip() {
        let text = "hello 世界";
        let bytes = encode_text(text, TextEncoding::Utf8, false);
        let decoded = detect_and_decode(&bytes);
        assert_eq!(decoded.text, text);
        assert_eq!(decoded.encoding, TextEncoding::Utf8);
    }

    #[test]
    fn gbk_decode() {
        let (encoded, _, _) = encoding_rs::GBK.encode("中文测试");
        let decoded = detect_and_decode(&encoded);
        assert_eq!(decoded.text, "中文测试");
        assert_eq!(decoded.encoding, TextEncoding::Gbk);
    }
}
