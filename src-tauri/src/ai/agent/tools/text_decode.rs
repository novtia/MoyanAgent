//! Decode raw file bytes into `String` for text-reading tools.
//!
//! Windows `.txt` files are often UTF-16 LE (Notepad / export tools) or
//! GBK/CP936 without a BOM — not UTF-8. [`decode_file_bytes`] picks the
//! encoding without requiring callers to know the on-disk format.

/// Decode file bytes for the `Read` tool and similar text consumers.
pub fn decode_file_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    // ── BOM ──────────────────────────────────────────────────────────────
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if bytes.starts_with(&[0xFF, 0xFE]) {
        if let Some(s) = decode_utf16_le(&bytes[2..]) {
            return s;
        }
    }
    if bytes.starts_with(&[0xFE, 0xFF]) {
        if let Some(s) = decode_utf16_be(&bytes[2..]) {
            return s;
        }
    }

    // ── UTF-8 (strict — only when the bytes are actually valid UTF-8) ──
    if std::str::from_utf8(bytes).is_ok() {
        return String::from_utf8_lossy(bytes).into_owned();
    }

    // ── UTF-16 LE without BOM (common for Windows `.txt`) ────────────────
    if bytes.len() % 2 == 0 {
        if let Some(s) = decode_utf16_le(bytes) {
            return s;
        }
        if let Some(s) = decode_utf16_be(bytes) {
            return s;
        }
    }

    // ── Windows legacy ANSI / GBK ────────────────────────────────────────
    #[cfg(windows)]
    {
        return decode_gbk(bytes);
    }

    #[cfg(not(windows))]
    {
        String::from_utf8_lossy(bytes).into_owned()
    }
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

/// Reject a UTF-16 decode that mostly produced U+FFFD (wrong endianness /
/// not actually UTF-16).
fn too_many_replacement_chars(s: &str) -> bool {
    let n = s.chars().count();
    if n == 0 {
        return false;
    }
    let bad = s.chars().filter(|&c| c == '\u{FFFD}').count();
    bad * 10 > n
}

/// GBK / CP936 on Chinese Windows — for legacy `.txt` saved as ANSI.
#[cfg(windows)]
fn decode_gbk(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }

    const CP936: u32 = 936;

    extern "system" {
        fn MultiByteToWideChar(
            code_page: u32,
            dw_flags: u32,
            lp_multi_byte_str: *const u8,
            cb_multi_byte: i32,
            lp_wide_char_str: *mut u16,
            cch_wide_char: i32,
        ) -> i32;
    }

    let len = bytes.len() as i32;
    unsafe {
        let needed =
            MultiByteToWideChar(CP936, 0, bytes.as_ptr(), len, std::ptr::null_mut(), 0);
        if needed <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut buf = vec![0u16; needed as usize];
        let written =
            MultiByteToWideChar(CP936, 0, bytes.as_ptr(), len, buf.as_mut_ptr(), needed);
        if written <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        String::from_utf16_lossy(&buf[..written as usize])
    }
}
