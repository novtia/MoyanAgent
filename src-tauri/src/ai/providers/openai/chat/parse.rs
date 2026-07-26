use serde_json::Value;

use crate::ai::chat::GenerateResponse;
use crate::ai::tokens;
use crate::error::{AppError, AppResult};

use super::super::common::{
    empty_response_details, extract_images, parse_response_json, parse_tool_call_arguments,
    top_level_error_message,
};

pub(crate) fn parse_openai_like_response(final_txt: &str) -> AppResult<GenerateResponse> {
    if final_txt.is_empty() {
        return Err(AppError::Upstream(
            "upstream returned an empty response body".into(),
        ));
    }

    let v: Value = parse_response_json(final_txt)?;
    if let Some(msg) = top_level_error_message(&v) {
        return Err(AppError::Upstream(msg));
    }

    let images = extract_images(&v);
    let text = extract_openai_chat_text(&v);
    let thinking_content = extract_openai_chat_thinking(&v);
    let tool_calls = extract_openai_chat_tool_calls(&v);
    let has_text = text.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    let has_thinking = thinking_content
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if images.is_empty() && !has_text && tool_calls.is_empty() && !has_thinking {
        return Err(AppError::Upstream(format!(
            "upstream response did not contain generated image, text, tool_calls, or reasoning. {}",
            empty_response_details(&v)
        )));
    }

    Ok(GenerateResponse {
        images,
        videos: Vec::new(),
        text,
        thinking_content,
        usage: tokens::extract_usage(&v),
        tool_calls,
        response_id: None,
    })
}
pub(crate) fn extract_openai_chat_tool_calls(v: &Value) -> Vec<crate::ai::chat::ProviderToolCall> {
    let Some(arr) = v
        .pointer("/choices/0/message/tool_calls")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|tc| {
            let id = tc.get("id").and_then(Value::as_str)?.to_string();
            let name = tc
                .pointer("/function/name")
                .and_then(Value::as_str)?
                .to_string();
            let raw_args = tc
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let arguments = parse_tool_call_arguments(&name, raw_args);
            Some(crate::ai::chat::ProviderToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
}

pub(crate) fn extract_openai_chat_text(v: &Value) -> Option<String> {
    let msg = v.pointer("/choices/0/message")?;

    if let Some(s) = msg.get("content").and_then(Value::as_str) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    if let Some(arr) = msg.get("content").and_then(Value::as_array) {
        let mut parts: Vec<String> = Vec::new();
        for it in arr {
            let typ = it.get("type").and_then(Value::as_str).unwrap_or("");
            if typ == "reasoning" {
                continue;
            }
            if let Some(s) = it.get("text").and_then(Value::as_str) {
                if !s.trim().is_empty() {
                    parts.push(s.trim().to_string());
                }
            }
        }
        if !parts.is_empty() {
            return Some(parts.join("\n\n"));
        }
    }

    None
}

pub(crate) fn extract_openai_chat_thinking(v: &Value) -> Option<String> {
    let msg = v.pointer("/choices/0/message")?;
    let mut parts: Vec<String> = Vec::new();
    if let Some(s) = msg.get("reasoning").and_then(Value::as_str) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(s) = msg.get("reasoning_content").and_then(Value::as_str) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(arr) = msg.get("content").and_then(Value::as_array) {
        for it in arr {
            if it.get("type").and_then(Value::as_str) != Some("reasoning") {
                continue;
            }
            if let Some(s) = it
                .get("text")
                .or_else(|| it.get("reasoning"))
                .and_then(Value::as_str)
            {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    parts.push(trimmed.to_string());
                }
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}
