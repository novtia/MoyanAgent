use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::ai::chat::{GenerateResponse, ImageResult, StreamDelta, TextDeltaCallback};
use crate::ai::tokens::TokenUsage;
use crate::error::{AppError, AppResult};

use super::debug::upstream_debug;
use super::images::collect_inline_data_urls;
use super::tool_args::{finalize_pending_tool_calls, PendingStreamToolCall};

pub(crate) fn set_streaming(body: &mut Value, include_usage: bool) {
    if let Some(map) = body.as_object_mut() {
        map.insert("stream".into(), Value::Bool(true));
        // Chat Completions (incl. Volcengine Ark / Doubao): streaming omits
        // token usage unless explicitly requested. The final SSE chunk then
        // carries `usage` with an empty `choices` array.
        // Docs: https://console.volcengine.com/ark/region:cn-beijing/docs/82379/1569618
        if include_usage {
            map.insert(
                "stream_options".into(),
                json!({ "include_usage": true }),
            );
        }
    }
}

pub(crate) fn is_json_response(resp: &reqwest::Response) -> bool {
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            let v = v.to_ascii_lowercase();
            v.contains("application/json") && !v.contains("text/event-stream")
        })
        .unwrap_or(false)
}

pub(crate) fn emit_final_text_if_needed(resp: &GenerateResponse, on_text_delta: &TextDeltaCallback) {
    if let Some(text) = resp.text.as_deref() {
        if !text.is_empty() {
            (on_text_delta)(StreamDelta::text(text.to_string()));
        }
    }
}
pub(crate) fn without_streaming(body: &Value) -> Value {
    let mut body = body.clone();
    if let Some(map) = body.as_object_mut() {
        map.remove("stream");
        map.remove("stream_options");
    }
    body
}
pub(crate) fn merge_usage(target: &mut TokenUsage, next: TokenUsage) {
    if next.prompt_tokens.is_some() {
        target.prompt_tokens = next.prompt_tokens;
    }
    if next.completion_tokens.is_some() {
        target.completion_tokens = next.completion_tokens;
    }
    if next.total_tokens.is_some() {
        target.total_tokens = next.total_tokens;
    }
    if next.cache_read_tokens.is_some() {
        target.cache_read_tokens = next.cache_read_tokens;
    }
    if next.cache_write_tokens.is_some() {
        target.cache_write_tokens = next.cache_write_tokens;
    }
}
pub(crate) fn finalize_stream_response(
    text: String,
    thinking: String,
    mut images: Vec<ImageResult>,
    usage: TokenUsage,
    pending_tool_calls: Vec<PendingStreamToolCall>,
) -> AppResult<GenerateResponse> {
    let tool_calls = finalize_pending_tool_calls(pending_tool_calls);
    if upstream_debug() {
        eprintln!(
            "[ATELIER_DEBUG_UPSTREAM] stream assembled: text_chars={} thinking_chars={} image_count={} tool_calls={} usage={:?}",
            text.chars().count(),
            thinking.chars().count(),
            images.len(),
            tool_calls.len(),
            usage
        );
    }
    if !text.is_empty() {
        collect_inline_data_urls(&text, &mut images);
    }
    let text = if text.trim().is_empty() {
        None
    } else {
        Some(text)
    };
    let thinking_content = if thinking.trim().is_empty() {
        None
    } else {
        Some(thinking)
    };
    if images.is_empty()
        && text.as_deref().map(str::is_empty).unwrap_or(true)
        && thinking_content
            .as_deref()
            .map(str::is_empty)
            .unwrap_or(true)
        && tool_calls.is_empty()
    {
        return Err(AppError::Upstream(
            "upstream stream did not contain generated image, text, or tool_calls".into(),
        ));
    }
    Ok(GenerateResponse {
        images,
        videos: Vec::new(),
        text,
        thinking_content,
        usage,
        tool_calls,
        response_id: None,
    })
}

/// Context wrapper for a transport error hit *while consuming* the SSE
/// response body. reqwest's own message here is usually the opaque
/// "error decoding response body"; [`crate::error::describe_reqwest_error`]
/// unfolds the source chain so the real cause (connection reset, unexpected
/// EOF, idle keepalive failure, ...) is visible in the UI.
pub(crate) fn stream_read_error(err: reqwest::Error) -> AppError {
    AppError::Upstream(format!(
        "connection interrupted while streaming upstream response: {}",
        crate::error::describe_reqwest_error(&err)
    ))
}
pub(crate) fn is_empty_stream_upstream_error(err: &AppError) -> bool {
    matches!(
        err,
        AppError::Upstream(s) if s.contains("upstream stream did not contain")
    )
}

pub(crate) fn upstream_rejects_streaming(status: StatusCode, msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    if !matches!(
        status,
        StatusCode::BAD_REQUEST
            | StatusCode::METHOD_NOT_ALLOWED
            | StatusCode::NOT_IMPLEMENTED
            | StatusCode::UNPROCESSABLE_ENTITY
    ) {
        return false;
    }
    // Ignore unknown-parameter complaints about `stream_options` — that means
    // we should drop the field, not disable streaming entirely.
    if m.contains("stream_options") {
        return false;
    }
    m.contains("stream") || m.contains("sse")
}
