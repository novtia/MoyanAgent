use reqwest::StatusCode;
use serde_json::Value;

use crate::ai::tokens;
use crate::ai::tokens::TokenUsage;
use crate::error::{AppError, AppResult};

pub(crate) fn upstream_error_message(txt: &str) -> String {
    match serde_json::from_str::<Value>(txt) {
        Ok(v) => v
            .pointer("/error/message")
            .or_else(|| v.pointer("/error/type"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| txt.to_string()),
        Err(_) => txt.to_string(),
    }
}
pub(crate) fn parse_response_json(txt: &str) -> AppResult<Value> {
    serde_json::from_str(txt).map_err(|err| {
        AppError::Upstream(format!(
            "failed to parse upstream JSON response: {}; body_bytes={}",
            err,
            txt.len()
        ))
    })
}

pub(crate) fn top_level_error_message(v: &Value) -> Option<String> {
    format_error_object(v.get("error"))
        .or_else(|| format_error_object(v.pointer("/choices/0/error")))
}

fn format_error_object(error: Option<&Value>) -> Option<String> {
    let error = error?;
    if let Some(s) = error.as_str() {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return None;
        }
        return Some(format!("upstream error: {}", trimmed));
    }
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.get("type").and_then(Value::as_str))
        .unwrap_or("unknown upstream error");
    let code = error
        .get("code")
        .or_else(|| error.get("type"))
        .map(|x| {
            x.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| x.to_string())
        })
        .filter(|s| !s.trim().is_empty() && s != "null");

    Some(match code {
        Some(code) => format!("upstream error {}: {}", code, message),
        None => format!("upstream error: {}", message),
    })
}

/// Transient upstream failures that are safe to retry with the same prompt.
///
/// OpenRouter often returns these as HTTP 200 + JSON/SSE `error` rather than
/// a 502/503/504 status, so callers must inspect the message, not just
/// [`reqwest::StatusCode`].
pub(crate) fn is_retryable_upstream_message(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    if is_idle_timeout_message(msg)
        || m.contains("gateway timeout")
        || m.contains("bad gateway")
        || m.contains("service unavailable")
        || m.contains("too many requests")
        || m.contains("temporarily unavailable")
        || m.contains("overloaded")
    {
        return true;
    }
    ["429", "502", "503", "504"].iter().any(|code| {
        m.contains(&format!("upstream error {code}")) || m.contains(&format!("http {code}"))
    })
}

pub(crate) fn is_idle_timeout_message(msg: &str) -> bool {
    msg.to_ascii_lowercase().contains("idle timeout")
}

/// Gemini 3+ can stream function-call arguments, but only when the request
/// sets `toolConfig.functionCallingConfig.streamFunctionCallArguments`.
/// Google AI Studio and some OpenRouter upstreams reject that field; the
/// caller strips it and retries rather than dropping tools entirely.
pub(crate) fn message_rejects_gemini_arg_streaming(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    m.contains("streamfunctioncallarguments")
        || m.contains("stream_function_call_arguments")
        || ((m.contains("toolconfig") || m.contains("tool_config"))
            && (m.contains("unknown")
                || m.contains("invalid")
                || m.contains("unrecognized")
                || m.contains("unexpected")))
}

pub(crate) fn upstream_rejects_gemini_arg_streaming(status: StatusCode, msg: &str) -> bool {
    matches!(status, StatusCode::BAD_REQUEST | StatusCode::NOT_FOUND)
        && message_rejects_gemini_arg_streaming(msg)
}

pub(crate) fn retryable_error_in_json_body(txt: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(txt) else {
        return false;
    };
    top_level_error_message(&v)
        .map(|msg| is_retryable_upstream_message(&msg))
        .unwrap_or(false)
}
pub(crate) fn push_response_detail(details: &mut Vec<String>, label: &str, value: &Value) {
    let raw = value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string());
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return;
    }
    let short: String = trimmed.chars().take(240).collect();
    details.push(format!("{}={}", label, short));
}

pub(crate) fn push_response_detail_at(details: &mut Vec<String>, v: &Value, path: &str, label: &str) {
    if let Some(value) = v.pointer(path) {
        push_response_detail(details, label, value);
    }
}

pub(crate) fn empty_response_details(v: &Value) -> String {
    let mut details = Vec::new();

    push_response_detail_at(&mut details, v, "/choices/0/finish_reason", "finish_reason");
    push_response_detail_at(
        &mut details,
        v,
        "/choices/0/native_finish_reason",
        "native_finish_reason",
    );
    push_response_detail_at(
        &mut details,
        v,
        "/choices/0/error/code",
        "choice_error_code",
    );
    push_response_detail_at(
        &mut details,
        v,
        "/choices/0/error/message",
        "choice_error_message",
    );
    push_response_detail_at(&mut details, v, "/choices/0/message/refusal", "refusal");
    push_response_detail_at(&mut details, v, "/error/code", "error_code");
    push_response_detail_at(&mut details, v, "/error/message", "error_message");
    push_response_detail_at(&mut details, v, "/status", "status");
    push_response_detail_at(
        &mut details,
        v,
        "/incomplete_details/reason",
        "incomplete_reason",
    );

    if v.pointer("/choices/0/message").is_none() && v.get("output").is_none() {
        details.push("missing choices[0].message or output".to_string());
    }

    if details.is_empty() {
        String::new()
    } else {
        format!("details: {}", details.join("; "))
    }
}

#[allow(dead_code)]
pub(crate) fn _usage_from_openai(v: &Value) -> TokenUsage {
    tokens::extract_usage(v)
}
