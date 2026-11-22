use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("db: {0}")]
    Db(#[from] rusqlite::Error),

    #[error("pool: {0}")]
    Pool(#[from] r2d2::Error),

    #[error("http: {0}")]
    Http(String),

    #[error("image: {0}")]
    Image(#[from] image::ImageError),

    #[error("base64: {0}")]
    Base64(#[from] base64::DecodeError),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("invalid argument: {0}")]
    Invalid(String),

    #[error("upstream: {0}")]
    Upstream(String),

    #[error("config: {0}")]
    Config(String),

    #[error("generation cancelled")]
    Canceled,

    #[error("{0}")]
    Other(String),
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        AppError::Http(describe_reqwest_error(&e))
    }
}

/// Render a reqwest error with its full `source()` chain plus classification
/// flags, so opaque top-level messages like "error decoding response body"
/// also reveal *why* (e.g. "connection closed before message completed",
/// "unexpected end of file", the underlying io/timeout error, ...).
pub fn describe_reqwest_error(e: &reqwest::Error) -> String {
    use std::error::Error as _;

    let mut parts: Vec<String> = vec![e.to_string()];
    let mut src = e.source();
    while let Some(cur) = src {
        let msg = cur.to_string();
        // Skip empty or duplicated links so the chain stays readable.
        if !msg.trim().is_empty() && parts.last().map(|p| p != &msg).unwrap_or(true) {
            parts.push(msg);
        }
        src = cur.source();
    }

    let mut kinds: Vec<String> = Vec::new();
    if e.is_timeout() {
        kinds.push("timeout".into());
    }
    if e.is_connect() {
        kinds.push("connect".into());
    }
    if e.is_body() {
        kinds.push("body".into());
    }
    if e.is_decode() {
        kinds.push("decode".into());
    }
    if e.is_request() {
        kinds.push("request".into());
    }
    if e.is_redirect() {
        kinds.push("redirect".into());
    }
    if let Some(status) = e.status() {
        kinds.push(format!("status={}", status.as_u16()));
    }

    let detail = parts.join(": ");
    if kinds.is_empty() {
        detail
    } else {
        format!("{detail} [{}]", kinds.join(","))
    }
}

/// Substrings (matched case-insensitively) that identify an abrupt transport
/// close: the peer dropped the connection without a clean TLS shutdown.
/// rustls reports this as `UnexpectedEof` ("peer closed connection without
/// sending TLS close_notify"); hyper/io surface it as connection reset or
/// early EOF. Such failures are transient by nature and safe to retry.
const ABRUPT_CLOSE_MARKERS: &[&str] = &[
    "close_notify",
    "unexpected end of file",
    "connection closed",
    "connection reset",
    "forcibly closed",
    "broken pipe",
    "early eof",
];

/// Whether a rendered error chain looks like an abrupt transport close.
pub fn msg_indicates_abrupt_close(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    ABRUPT_CLOSE_MARKERS.iter().any(|marker| m.contains(marker))
}

/// Whether a reqwest error's full source chain indicates the peer closed the
/// connection abruptly (no TLS close_notify, reset, early EOF, ...).
pub fn reqwest_error_indicates_abrupt_close(err: &reqwest::Error) -> bool {
    msg_indicates_abrupt_close(&describe_reqwest_error(err))
}

/// Substrings (matched case-insensitively) that identify a request rejected for
/// exceeding the model's context window. Upstreams all report this as a plain
/// HTTP 400 with no machine-readable code, so the prose is the only signal.
const CONTEXT_OVERFLOW_MARKERS: &[&str] = &[
    "maximum context length",
    "context length exceeded",
    "context_length_exceeded",
    "reduce the length of the messages",
    "exceed context limit",
    "prompt is too long",
    "input is too long",
    "too many tokens",
];

/// Whether an upstream error message says the request did not fit the model's
/// context window. Callers use it to shrink the conversation and retry rather
/// than surfacing a dead end to the user.
pub fn msg_indicates_context_overflow(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    CONTEXT_OVERFLOW_MARKERS
        .iter()
        .any(|marker| m.contains(marker))
}

/// Numbers recovered from a context-length rejection.
///
/// The rejection is the only place an upstream ever states its real window, and
/// the only ground truth for how far the local character-based estimate drifts
/// from the model's tokenizer. Both are worth keeping: the window because a
/// model missing from the catalog otherwise has none, and the message count
/// because it says by how much the next attempt has to undershoot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ContextOverflowReport {
    /// Window the upstream says the model has.
    pub context_window: Option<i64>,
    /// Total tokens it counted for the rejected request (messages + completion).
    pub requested_tokens: Option<i64>,
    /// Tokens it counted for the messages alone.
    pub message_tokens: Option<i64>,
}

/// How far after a marker we will look for its number. Upstream phrasings put
/// it immediately after; a wider scan risks latching onto an unrelated figure
/// later in the sentence.
const MARKER_SCAN_CHARS: usize = 48;

/// Read the first integer that follows `marker`.
fn int_after(haystack: &str, marker: &str) -> Option<i64> {
    let start = haystack.find(marker)? + marker.len();
    let tail: String = haystack[start..].chars().take(MARKER_SCAN_CHARS).collect();
    let digits: String = tail
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// Read the integer that immediately precedes `marker`.
///
/// DeepSeek reports the split as `(939986 in the messages, 128000 in the
/// completion)`, so the figure sits *before* its label.
fn int_before(haystack: &str, marker: &str) -> Option<i64> {
    let end = haystack.find(marker)?;
    let head = &haystack[..end];
    let window: Vec<char> = head
        .chars()
        .rev()
        .take(MARKER_SCAN_CHARS)
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    if window.is_empty() {
        return None;
    }
    let digits: String = window.into_iter().rev().collect();
    digits.parse().ok()
}

/// Extract whatever numbers an upstream context-length rejection stated.
///
/// Returns `None` when the message is not a context-length rejection at all, so
/// callers can distinguish "not this kind of error" from "this kind of error but
/// it named no numbers".
pub fn parse_context_overflow(msg: &str) -> Option<ContextOverflowReport> {
    if !msg_indicates_context_overflow(msg) {
        return None;
    }
    let m = msg.to_ascii_lowercase();
    let context_window = int_after(&m, "maximum context length is")
        .or_else(|| int_after(&m, "maximum context length of"))
        .or_else(|| int_after(&m, "context length is"))
        .or_else(|| int_after(&m, "maximum input length is"))
        .or_else(|| int_after(&m, "context limit is"))
        .filter(|w| *w > 0);
    Some(ContextOverflowReport {
        context_window,
        requested_tokens: int_after(&m, "you requested").filter(|t| *t > 0),
        message_tokens: int_before(&m, "in the messages").filter(|t| *t > 0),
    })
}

/// [`parse_context_overflow`] applied to an error's rendered chain.
pub fn error_context_overflow_report(err: &AppError) -> Option<ContextOverflowReport> {
    if !matches!(
        err,
        AppError::Upstream(_) | AppError::Http(_) | AppError::Other(_)
    ) {
        return None;
    }
    parse_context_overflow(&err.to_string())
}

/// Substrings (matched case-insensitively) that identify an upstream HTTP 429 /
/// rate-limit / resource-exhausted rejection.
const RATE_LIMIT_MARKERS: &[&str] = &[
    "http 429",
    "status=429",
    "error 429",
    "too many requests",
    "resource exhausted",
    "resource_exhausted",
    "rate limit",
    "rate_limit",
];

/// Whether a rendered error message indicates an HTTP 429 / rate-limit failure.
pub fn msg_indicates_rate_limit(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    RATE_LIMIT_MARKERS.iter().any(|marker| m.contains(marker))
}

/// Whether an [`AppError`] represents an upstream HTTP 429 / rate-limit error
/// that is safe to retry inside the agent loop.
pub fn error_is_rate_limited(err: &AppError) -> bool {
    if !matches!(
        err,
        AppError::Upstream(_) | AppError::Http(_) | AppError::Other(_)
    ) {
        return false;
    }
    msg_indicates_rate_limit(&err.to_string())
}

#[cfg(test)]
mod rate_limit_tests {
    use super::*;

    #[test]
    fn vertex_429_is_detected() {
        let err = AppError::Upstream(
            "Vertex AI (vertex) HTTP 429 Too Many Requests: Resource exhausted. \
             Please try again later. Please refer to \
             https://cloud.google.com/vertex-ai/generative-ai/docs/error-code-429 \
             for more details."
                .into(),
        );
        assert!(error_is_rate_limited(&err));
    }

    #[test]
    fn openrouter_and_http_429_are_detected() {
        assert!(error_is_rate_limited(&AppError::Upstream(
            "upstream error 429: rate limit exceeded".into()
        )));
        assert!(error_is_rate_limited(&AppError::Http(
            "request failed [status=429]".into()
        )));
    }

    #[test]
    fn non_rate_limit_errors_are_not_detected() {
        assert!(!error_is_rate_limited(&AppError::Canceled));
        assert!(!error_is_rate_limited(&AppError::Invalid("429".into())));
        assert!(!error_is_rate_limited(&AppError::Upstream(
            "HTTP 401 Unauthorized: invalid api key".into()
        )));
        assert!(!error_is_rate_limited(&AppError::Upstream(
            "HTTP 400 Bad Request: maximum context length exceeded".into()
        )));
    }
}

#[cfg(test)]
mod context_overflow_tests {
    use super::*;

    /// The exact rejection this parser exists for.
    #[test]
    fn deepseek_rejection_is_fully_parsed() {
        let msg = "upstream: DeepSeek (deepseek) HTTP 400 Bad Request: This model's \
                   maximum context length is 1048576 tokens. However, you requested \
                   1067986 tokens (939986 in the messages, 128000 in the completion). \
                   Please reduce the length of the messages or completion.";
        let r = parse_context_overflow(msg).expect("recognised as an overflow");
        assert_eq!(r.context_window, Some(1_048_576));
        assert_eq!(r.requested_tokens, Some(1_067_986));
        assert_eq!(r.message_tokens, Some(939_986));
    }

    #[test]
    fn openai_phrasing_yields_the_window() {
        let msg = "upstream: HTTP 400: This model's maximum context length is 128000 \
                   tokens, however you requested 131000 tokens.";
        let r = parse_context_overflow(msg).expect("recognised");
        assert_eq!(r.context_window, Some(128_000));
        assert_eq!(r.requested_tokens, Some(131_000));
        assert_eq!(r.message_tokens, None);
    }

    /// Anthropic names no numbers; the caller still needs to know it *was* an
    /// overflow so it shrinks instead of surfacing the error.
    #[test]
    fn recognised_without_numbers() {
        let r = parse_context_overflow("upstream: prompt is too long").expect("recognised");
        assert_eq!(r, ContextOverflowReport::default());
    }

    #[test]
    fn unrelated_errors_are_not_overflows() {
        assert!(parse_context_overflow("upstream: HTTP 401 invalid api key").is_none());
    }

    /// A stray figure far from the marker must not be mistaken for the window.
    #[test]
    fn distant_numbers_are_not_captured() {
        let msg = "upstream: context length exceeded. See docs, then retry after \
                   waiting a while and eventually maybe 42 seconds";
        let r = parse_context_overflow(msg).expect("recognised");
        assert_eq!(r.context_window, None);
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        AppError::Other(format!("{e:#}"))
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        AppError::Other(e.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        s.serialize_str(&self.to_string())
    }
}

pub type AppResult<T> = Result<T, AppError>;
