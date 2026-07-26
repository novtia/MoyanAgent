

pub(crate) fn find_sse_event_end(buffer: &[u8]) -> Option<(usize, usize)> {
    for i in 0..buffer.len().saturating_sub(3) {
        if &buffer[i..i + 4] == b"\r\n\r\n" {
            return Some((i, 4));
        }
    }
    for i in 0..buffer.len().saturating_sub(1) {
        if &buffer[i..i + 2] == b"\n\n" {
            return Some((i, 2));
        }
    }
    None
}

pub(crate) fn sse_data_payload(event: &str) -> Option<String> {
    sse_event_name_and_data(event).map(|(_, data)| data)
}

/// Parse SSE `event:` / `data:` lines. Ark may put the event type only on the
/// `event:` line while `data:` omits `type`.
pub(crate) fn sse_event_name_and_data(event: &str) -> Option<(Option<String>, String)> {
    let mut event_name = None;
    let mut data = Vec::new();
    for raw_line in event.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if let Some(rest) = line.strip_prefix("event:") {
            let name = rest.strip_prefix(' ').unwrap_or(rest).trim();
            if !name.is_empty() {
                event_name = Some(name.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("data:") {
            data.push(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
    if data.is_empty() {
        None
    } else {
        Some((event_name, data.join("\n")))
    }
}
