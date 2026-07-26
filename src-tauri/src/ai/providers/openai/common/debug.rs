use serde_json::Value;

/// When `ATELIER_DEBUG_UPSTREAM` is `1` / `true` / `yes`, eprintln request JSON
/// bodies and response bodies (truncated for large payloads) for OpenAI-compatible calls.
pub(crate) fn upstream_debug() -> bool {
    matches!(
        std::env::var("ATELIER_DEBUG_UPSTREAM").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

pub(crate) fn debug_log_upstream_request(label: &str, endpoint: &str, body: &Value) {
    if !upstream_debug() {
        return;
    }
    let body_str = serde_json::to_string_pretty(body).unwrap_or_else(|_| body.to_string());
    eprintln!(
        "[ATELIER_DEBUG_UPSTREAM] {} POST {}\n{}",
        label, endpoint, body_str
    );
}

pub(crate) fn debug_log_upstream_response_text(label: &str, txt: &str) {
    if !upstream_debug() {
        return;
    }
    pub(crate) const MAX: usize = 16_384;
    if txt.len() <= MAX {
        eprintln!("[ATELIER_DEBUG_UPSTREAM] {} response JSON:\n{}", label, txt);
    } else {
        eprintln!(
            "[ATELIER_DEBUG_UPSTREAM] {} response JSON (truncated, total {} bytes):\n{}…",
            label,
            txt.len(),
            &txt[..MAX]
        );
    }
}

/// Logs the first `max` complete SSE events (useful to see whether `reasoning` / `reasoning_text` deltas appear).
pub(crate) fn debug_log_sse_event(emitted: &mut u32, max: u32, event: &str) {
    if !upstream_debug() || *emitted >= max {
        return;
    }
    *emitted += 1;
    let preview: String = event.chars().take(1200).collect();
    let suffix = if event.len() > 1200 { "…" } else { "" };
    eprintln!(
        "[ATELIER_DEBUG_UPSTREAM] SSE event {}/{} (preview):\n{}{}",
        *emitted, max, preview, suffix
    );
}
