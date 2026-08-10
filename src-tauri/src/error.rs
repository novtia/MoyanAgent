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
