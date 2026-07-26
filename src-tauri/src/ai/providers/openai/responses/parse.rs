use serde_json::Value;

use crate::ai::chat::GenerateResponse;
use crate::ai::tokens;
use crate::error::{AppError, AppResult};

use super::super::common::{
    collect_response_images, empty_response_details, parse_response_json, top_level_error_message,
};
use serde_json::json;

pub(crate) fn extract_responses_tool_calls(v: &Value) -> Vec<crate::ai::chat::ProviderToolCall> {
    let Some(arr) = v.get("output").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|item| {
            let typ = item.get("type").and_then(Value::as_str).unwrap_or("");
            if typ != "function_call" {
                return None;
            }
            let id = item
                .get("call_id")
                .and_then(Value::as_str)
                .or_else(|| item.get("id").and_then(Value::as_str))?
                .to_string();
            let name = item.get("name").and_then(Value::as_str)?.to_string();
            let raw_args = item
                .get("arguments")
                .and_then(|a| {
                    if let Some(s) = a.as_str() {
                        Some(s.to_string())
                    } else {
                        Some(a.to_string())
                    }
                })
                .unwrap_or_else(|| "{}".into());
            let arguments = serde_json::from_str(&raw_args).unwrap_or_else(|_| json!({}));
            Some(crate::ai::chat::ProviderToolCall {
                id,
                name,
                arguments,
            })
        })
        .collect()
}
pub(crate) fn parse_responses_response(final_txt: &str) -> AppResult<GenerateResponse> {
    if final_txt.is_empty() {
        return Err(AppError::Upstream(
            "upstream returned an empty response body".into(),
        ));
    }

    let v: Value = parse_response_json(final_txt)?;
    if let Some(msg) = top_level_error_message(&v) {
        return Err(AppError::Upstream(msg));
    }

    let mut images = Vec::new();
    collect_response_images(&v, &mut images);
    let text = extract_responses_text(&v);
    let thinking_content = extract_responses_reasoning(&v);
    let tool_calls = extract_responses_tool_calls(&v);
    let response_id = v
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let has_text = text.as_deref().map(|s| !s.is_empty()).unwrap_or(false);
    let has_thinking = thinking_content
        .as_deref()
        .map(|s| !s.is_empty())
        .unwrap_or(false);
    if images.is_empty() && !has_text && !has_thinking && tool_calls.is_empty() {
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
        response_id,
    })
}
pub(crate) fn extract_responses_text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("output_text").and_then(Value::as_str) {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    let mut parts = Vec::new();
    collect_text_values(v.get("output").unwrap_or(v), &mut parts);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub(crate) fn extract_responses_reasoning(v: &Value) -> Option<String> {
    let mut parts = Vec::new();
    collect_reasoning_values(v.get("output").unwrap_or(v), &mut parts);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    }
}

pub(crate) fn collect_reasoning_values(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(items) => {
            for item in items {
                collect_reasoning_values(item, out);
            }
        }
        Value::Object(map) => {
            let typ = map.get("type").and_then(Value::as_str).unwrap_or("");
            // Reasoning item: `{ type: "reasoning", summary: [{type:summary_text,text}], content? }`
            if typ == "reasoning" || typ.starts_with("reasoning") {
                push_trimmed_text(map.get("text"), out);
                if let Some(summary) = map.get("summary") {
                    collect_reasoning_text_parts(summary, out);
                }
                if let Some(content) = map.get("content") {
                    collect_reasoning_text_parts(content, out);
                }
                return;
            }
            if matches!(typ, "summary_text" | "reasoning_text") {
                push_trimmed_text(map.get("text"), out);
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_reasoning_text_parts(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(items) => {
            for item in items {
                collect_reasoning_text_parts(item, out);
            }
        }
        Value::Object(map) => {
            let typ = map.get("type").and_then(Value::as_str).unwrap_or("");
            if matches!(typ, "summary_text" | "reasoning_text" | "") {
                push_trimmed_text(map.get("text"), out);
            }
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_string());
            }
        }
        _ => {}
    }
}

pub(crate) fn push_trimmed_text(v: Option<&Value>, out: &mut Vec<String>) {
    if let Some(text) = v.and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()) {
        out.push(text.to_string());
    }
}

pub(crate) fn collect_text_values(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(items) => {
            for item in items {
                collect_text_values(item, out);
            }
        }
        Value::Object(map) => {
            let typ = map.get("type").and_then(Value::as_str).unwrap_or("");
            // Reasoning items / summary_text belong to thinking, never assistant body.
            if typ == "reasoning" || typ.starts_with("reasoning") || typ == "summary_text" {
                return;
            }
            if matches!(typ, "output_text" | "text") {
                push_trimmed_text(map.get("text"), out);
            }
            if let Some(content) = map.get("content") {
                collect_text_values(content, out);
            }
        }
        _ => {}
    }
}
