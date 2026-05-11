use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use serde_json::{json, Map, Value};

use crate::ai::chat::{AttachmentBytes, ChatRequest, GenerateResponse, HistoryTurn};
use crate::ai::providers::{ChatProvider, ProviderFuture, CLAUDE_SDK};
use crate::ai::tokens::TokenUsage;
use crate::error::{AppError, AppResult};

const UPSTREAM_TIMEOUT_SECS: u64 = 15 * 60;
const ANTHROPIC_VERSION: &str = "2023-06-01";
const DEFAULT_MAX_TOKENS: i64 = 4096;

pub struct ClaudeProvider;

impl ClaudeProvider {
    pub fn new() -> Self {
        Self
    }
}

impl ChatProvider for ClaudeProvider {
    fn sdk(&self) -> &'static str {
        CLAUDE_SDK
    }

    fn chat<'a>(&'a self, request: ChatRequest) -> ProviderFuture<'a> {
        Box::pin(async move { generate(request).await })
    }
}

async fn generate(request: ChatRequest) -> AppResult<GenerateResponse> {
    let body = build_body(&request);
    let provider_label = provider_label(&request);

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(UPSTREAM_TIMEOUT_SECS))
        .build()?;

    let resp = client
        .post(&request.provider.endpoint)
        .header("Content-Type", "application/json")
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("x-api-key", &request.provider.api_key)
        .json(&body)
        .send()
        .await?;

    let status = resp.status();
    let txt = resp.text().await?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "{} HTTP {}: {}",
            provider_label,
            status,
            upstream_error_message(&txt)
        )));
    }
    parse_response(&txt)
}

fn provider_label(request: &ChatRequest) -> String {
    if request.provider.name.trim().is_empty() {
        request.provider.id.clone()
    } else {
        format!("{} ({})", request.provider.name, request.provider.id)
    }
}

fn build_body(request: &ChatRequest) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    for turn in &request.history {
        if let Some(message) = history_turn_to_message(turn) {
            messages.push(message);
        }
    }
    messages.push(message_from_parts(
        "user",
        Some(&request.prompt),
        &request.attachments,
    ));

    // Thread the prior assistant tool_use turn + its tool_result
    // replies through the message stream. Anthropic strictly requires
    // call/response symmetry.
    if let Some(pending) = &request.pending_assistant_turn {
        let mut content: Vec<Value> = Vec::new();
        if let Some(text) = pending.text.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            content.push(json!({ "type": "text", "text": text }));
        }
        for tc in &pending.tool_calls {
            content.push(json!({
                "type": "tool_use",
                "id": tc.id,
                "name": tc.name,
                "input": tc.arguments,
            }));
        }
        if !content.is_empty() {
            messages.push(json!({ "role": "assistant", "content": content }));
        }
    }
    if !request.tool_results.is_empty() {
        let blocks: Vec<Value> = request
            .tool_results
            .iter()
            .map(|tr| {
                let content = match &tr.content {
                    Value::String(s) => Value::String(s.clone()),
                    other => Value::String(other.to_string()),
                };
                let mut b = json!({
                    "type": "tool_result",
                    "tool_use_id": tr.tool_call_id,
                    "content": content,
                });
                if tr.is_error {
                    b.as_object_mut().unwrap().insert("is_error".into(), Value::Bool(true));
                }
                b
            })
            .collect();
        messages.push(json!({ "role": "user", "content": blocks }));
    }

    let mut body = json!({
        "model": request.model,
        "max_tokens": request.parameters.model.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "messages": messages,
    });
    let map = body.as_object_mut().unwrap();
    let sys = request.system_prompt.trim();
    if !sys.is_empty() {
        map.insert("system".into(), Value::String(sys.to_string()));
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.schema,
                })
            })
            .collect();
        map.insert("tools".into(), Value::Array(tools));
    }
    apply_params(map, request);
    body
}

fn history_turn_to_message(turn: &HistoryTurn) -> Option<Value> {
    let text = turn.text.as_deref();
    if text.map(|s| s.trim().is_empty()).unwrap_or(true) && turn.images.is_empty() {
        return None;
    }
    let role = if turn.role == "assistant" {
        "assistant"
    } else {
        "user"
    };
    let images: &[AttachmentBytes] = if role == "user" { &turn.images } else { &[] };
    Some(message_from_parts(role, text, images))
}

fn message_from_parts(role: &str, text: Option<&str>, attachments: &[AttachmentBytes]) -> Value {
    if attachments.is_empty() {
        return json!({ "role": role, "content": text.unwrap_or_default() });
    }

    let mut content: Vec<Value> = Vec::new();
    for attachment in attachments {
        content.push(json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": attachment.mime.as_str(),
                "data": B64.encode(&attachment.bytes),
            }
        }));
    }
    if let Some(text) = text {
        if !text.trim().is_empty() {
            content.push(json!({ "type": "text", "text": text }));
        }
    }
    json!({ "role": role, "content": content })
}

fn apply_params(map: &mut Map<String, Value>, request: &ChatRequest) {
    if let Some(v) = request.parameters.model.temperature {
        map.insert("temperature".into(), json!(v));
    }
    if let Some(v) = request.parameters.model.top_p {
        map.insert("top_p".into(), json!(v));
    }
}

fn parse_response(txt: &str) -> AppResult<GenerateResponse> {
    if txt.is_empty() {
        return Err(AppError::Upstream(
            "upstream returned an empty response body".into(),
        ));
    }
    let v: Value = serde_json::from_str(txt).map_err(|err| {
        AppError::Upstream(format!(
            "failed to parse upstream JSON response: {}; body_bytes={}",
            err,
            txt.len()
        ))
    })?;
    if let Some(msg) = v.pointer("/error/message").and_then(Value::as_str) {
        return Err(AppError::Upstream(msg.to_string()));
    }

    let mut parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<crate::ai::chat::ProviderToolCall> = Vec::new();
    if let Some(content) = v.get("content").and_then(Value::as_array) {
        for item in content {
            match item.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(text) = item.get("text").and_then(Value::as_str) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            parts.push(trimmed.to_string());
                        }
                    }
                }
                Some("tool_use") => {
                    let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
                    let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                    if id.is_empty() || name.is_empty() {
                        continue;
                    }
                    let input = item.get("input").cloned().unwrap_or(Value::Null);
                    tool_calls.push(crate::ai::chat::ProviderToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
                        arguments: input,
                    });
                }
                _ => {}
            }
        }
    }

    let text = if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n\n"))
    };

    if text.is_none() && tool_calls.is_empty() {
        return Err(AppError::Upstream(format!(
            "upstream response did not contain generated text or tool_use. {}",
            empty_response_details(&v)
        )));
    }

    Ok(GenerateResponse {
        images: Vec::new(),
        text,
        usage: usage(&v),
        tool_calls,
    })
}

fn usage(v: &Value) -> TokenUsage {
    let usage = v.get("usage").unwrap_or(&Value::Null);
    let prompt = usage.get("input_tokens").and_then(Value::as_i64);
    let completion = usage.get("output_tokens").and_then(Value::as_i64);
    TokenUsage {
        prompt_tokens: prompt,
        completion_tokens: completion,
        total_tokens: match (prompt, completion) {
            (Some(a), Some(b)) => Some(a + b),
            _ => None,
        },
    }
}

fn upstream_error_message(txt: &str) -> String {
    match serde_json::from_str::<Value>(txt) {
        Ok(v) => v
            .pointer("/error/message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| txt.to_string()),
        Err(_) => txt.to_string(),
    }
}

fn empty_response_details(v: &Value) -> String {
    let mut details = Vec::new();
    if let Some(stop_reason) = v.get("stop_reason").and_then(Value::as_str) {
        details.push(format!("stop_reason={stop_reason}"));
    }
    if v.get("content").is_none() {
        details.push("missing content".to_string());
    }
    if details.is_empty() {
        String::new()
    } else {
        format!("details: {}", details.join("; "))
    }
}
